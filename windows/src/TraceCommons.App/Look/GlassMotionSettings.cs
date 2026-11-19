using System;
using Microsoft.UI.Dispatching;
using Windows.UI.ViewManagement;

namespace TraceCommons.App.Look;

/// <summary>
/// Whether things may move, followed live (spec W5, checklist P1): the
/// system's "Animation effects" (<c>UISettings.AnimationsEnabled</c>, read
/// again whenever the system reports a change) and whether the main window
/// can be seen (<see cref="WindowClocks"/>). Every animation in
/// <see cref="TraceCommons.App.Look"/> asks <see cref="MayMove"/> before it
/// starts and listens to <see cref="Changed"/> to stop or restart.
/// </summary>
internal static class GlassMotionSettings
{
    private static readonly UISettings Settings = new();
    private static DispatcherQueue? _queue;
    private static bool _animationsEnabled = Settings.AnimationsEnabled;
    private static bool _listening;

    /// <summary>The system lets things move.</summary>
    public static bool AnimationsEnabled
    {
        get
        {
            Listen();
            return _animationsEnabled;
        }
    }

    /// <summary>Animations may run: the system allows them and the window can be seen.</summary>
    public static bool MayMove => AnimationsEnabled && WindowClocks.Running;

    /// <summary>Raised on the UI thread when <see cref="AnimationsEnabled"/> or the window's visibility changes.</summary>
    public static event Action? Changed;

    private static void Listen()
    {
        if (_listening)
        {
            return;
        }

        _listening = true;
        _queue = DispatcherQueue.GetForCurrentThread();
        WindowClocks.Changed += _ => Changed?.Invoke();
        // UISettings.AnimationsEnabledChanged is raised off the UI thread;
        // where the system lacks the event the setting is still read on
        // every start, and again whenever the window is shown.
        if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 19041))
        {
            Settings.AnimationsEnabledChanged += (_, _) => _queue?.TryEnqueue(Refresh);
        }
    }

    /// <summary>Reads the system setting again and tells listeners when it changed.</summary>
    public static void Refresh()
    {
        bool now = Settings.AnimationsEnabled;
        if (now != _animationsEnabled)
        {
            _animationsEnabled = now;
            Changed?.Invoke();
        }
    }
}
