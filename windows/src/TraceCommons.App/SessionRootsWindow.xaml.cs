using System;
using System.Collections.Generic;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// The roots window.
/// </summary>
/// <remarks>
/// Thin by design, like <see cref="OnboardingWindow"/>: every decision lives
/// in <see cref="SessionRootsViewModel"/>, and the rules worth testing live
/// below that again in the interop assembly, which builds on machines this
/// project cannot. This file only wires clicks.
/// </remarks>
public sealed partial class SessionRootsWindow : Window
{
    /// <param name="host">The daemon this window will start.</param>
    /// <param name="candidates">
    /// What discovery found, probed off the UI thread by the caller. Passed in
    /// rather than probed here because counting session files recursively is
    /// slow enough to hang a window that is being constructed.
    /// </param>
    public SessionRootsWindow(DaemonHost host, IReadOnlyList<SourceCandidate> candidates)
    {
        InitializeComponent();

        ViewModel = new SessionRootsViewModel(host, candidates);
        ViewModel.Finished += OnFinished;
    }

    /// <summary>
    /// Raised once the contributor has answered and the daemon has started.
    /// </summary>
    /// <remarks>
    /// The caller needs this rather than just the window closing: the queue
    /// window came up with no daemon behind it and has to load its first
    /// snapshot now that there is one.
    /// </remarks>
    public event Action? Declared;

    public SessionRootsViewModel ViewModel { get; }

    private void OnWatch(object sender, RoutedEventArgs e)
    {
        if (sender is Button { DataContext: SourceRowViewModel row })
        {
            row.ChooseWatch();
        }
    }

    private void OnDoNotUse(object sender, RoutedEventArgs e)
    {
        if (sender is Button { DataContext: SourceRowViewModel row })
        {
            row.ChooseOff();
        }
    }

    private async void OnContinue(object sender, RoutedEventArgs e) =>
        await ViewModel.ContinueAsync();

    private void OnFinished()
    {
        Declared?.Invoke();
        Close();
    }
}
