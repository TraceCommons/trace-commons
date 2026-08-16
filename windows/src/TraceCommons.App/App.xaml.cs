using System;
using Microsoft.UI.Xaml;

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

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _window = new MainWindow();
        _window.Activate();
    }
}
