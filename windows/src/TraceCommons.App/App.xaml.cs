using System;
using Microsoft.UI.Xaml;
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
    /// Read here rather than in the window because the command line belongs
    /// to the process. Every argument is offered to the parser, including
    /// this executable's own path, so the parser answers null rather than
    /// throwing for anything that is not an invite.
    ///
    /// Never logged. It is a credential, and invites are reusable.
    /// </remarks>
    internal static string? PendingInvite { get; private set; }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        // Unpackaged, so the URL arrives as argv rather than through a
        // packaged activation. Registering the scheme every launch keeps it
        // correct after the folder is moved, which an unpackaged app in a
        // folder someone keeps is free to be.
        UrlSchemeRegistration.EnsureRegistered();

        foreach (string argument in Environment.GetCommandLineArgs())
        {
            string? invite = DeepLink.InviteFrom(argument);
            if (invite is not null)
            {
                PendingInvite = invite;
                break;
            }
        }

        _window = new MainWindow();
        _window.Activate();
    }
}
