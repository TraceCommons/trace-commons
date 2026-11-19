using System;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// Whether a window's clocks may run: false while it is hidden to the tray,
/// minimised or closed. <see cref="Running"/> and <see cref="Changed"/> are
/// the main window's, set from its <see cref="WindowClockGate"/>, for the
/// controls with a timer of their own that live there (the managed-sessions
/// poll). <see cref="Windows"/> holds every window's answer by its XamlRoot,
/// so an animation asks about the window it is drawn in
/// (AnimationTimeline.swift: a timeline pauses unless its own window is
/// visible): the Settings window keeps moving while the main window sits in
/// the tray, and a minimised Settings window stops its own clocks.
/// </summary>
internal static class WindowClocks
{
    /// <summary>Every window's gate, by the XamlRoot its elements report.</summary>
    public static readonly WindowClockRegistry<XamlRoot> Windows = new();

    public static bool Running { get; private set; } = true;

    public static event Action<bool>? Changed;

    /// <summary>The main window's gate, with the root its content is drawn in.</summary>
    public static void Set(bool running, XamlRoot? root)
    {
        if (root is not null)
        {
            Windows.Set(root, running);
        }

        if (Running == running)
        {
            return;
        }

        Running = running;
        Changed?.Invoke(running);
    }

    /// <summary>
    /// Follows a window other than the main one with its own
    /// <see cref="WindowClockGate"/>: minimised from its presenter, hidden
    /// from its visibility, and dropped when it closes.
    /// </summary>
    public static void Watch(Window window)
    {
        var gate = new WindowClockGate();
        XamlRoot? root = null;
        void Report(bool mayTick)
        {
            root ??= window.Content?.XamlRoot;
            if (root is not null)
            {
                Windows.Set(root, mayTick);
            }
        }

        gate.Changed += Report;
        if (window.Content is FrameworkElement content)
        {
            content.Loaded += (_, _) => Report(gate.MayTick);
        }

        window.AppWindow.Changed += (sender, args) =>
        {
            if (args.DidPresenterChange || args.DidSizeChange)
            {
                gate.SetMinimised(sender.Presenter is OverlappedPresenter presenter
                    && presenter.State == OverlappedPresenterState.Minimized);
            }
        };
        window.VisibilityChanged += (_, args) => gate.SetHiddenToTray(!args.Visible);
        window.Closed += (_, _) =>
        {
            gate.SetClosed();
            if (root is not null)
            {
                Windows.Forget(root);
            }
        };
    }
}
