using System;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// Application entry point.
///
/// Deliberately thin: it creates the window and gets out of the way. The
/// daemon's lifetime belongs to <see cref="DaemonHost"/>, owned by the window,
/// so that "the app is running" and "the daemon is running" stay separable --
/// the window can report a daemon that failed to start instead of the process
/// dying at launch.
/// </summary>
public partial class App : Application
{
    private MainWindow? _window;

    public App()
    {
        InitializeComponent();
    }

    /// <summary>
    /// The invite from a <c>tracecommons://</c> deep link this process was
    /// launched with, if any.
    /// </summary>
    /// <remarks>
    /// MSIX protocol activations carry their URI through AppInstance rather
    /// than argv. Unpackaged launches retain command-line parsing. Both use
    /// the existing deep-link validator and only prefill Connect.
    ///
    /// Never logged. It is a credential, and invites are reusable.
    /// </remarks>
    internal static string? PendingInvite { get; private set; }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        // Packaged installs use manifest registration; this helper only
        // registers the scheme for unpackaged installs.
        UrlSchemeRegistration.EnsureRegistered();

        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        string? protocolUri = activation.Kind == ExtendedActivationKind.Protocol
            ? (activation.Data as Windows.ApplicationModel.Activation.IProtocolActivatedEventArgs)?.Uri?.AbsoluteUri ?? string.Empty
            : null;
        PendingInvite = DeepLink.InitialInvite(protocolUri, Environment.GetCommandLineArgs());

        _window = new MainWindow();
        _window.Activate();
    }
}
