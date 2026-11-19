using System;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Windows.UI.ViewManagement;

namespace TraceCommons.App.Look;

/// <summary>
/// Whether things may move, followed live (spec W5, checklist P1): the
/// system's "Animation effects" (<c>UISettings.AnimationsEnabled</c>, read
/// again whenever the system reports a change) and whether the element's
/// own window can be seen (<see cref="WindowClocks.Windows"/>) with the
/// element and every ancestor visible. Every animation in
/// <see cref="TraceCommons.App.Look"/> asks <see cref="MayMoveFor"/> before
/// it starts and listens to <see cref="Changed"/> to stop or restart.
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

    /// <summary>
    /// Whether <paramref name="element"/> may move now: the system allows it,
    /// its own window can be seen, and it and every ancestor are visible.
    /// </summary>
    public static bool MayMoveFor(UIElement element) =>
        AnimationsEnabled
        && WindowClocks.Windows.MayTick(element.XamlRoot)
        && AncestorVisibility.IsShown(element);

    /// <summary>Raised on the UI thread when <see cref="AnimationsEnabled"/> or any window's visibility changes.</summary>
    public static event Action? Changed;

    private static void Listen()
    {
        if (_listening)
        {
            return;
        }

        _listening = true;
        _queue = DispatcherQueue.GetForCurrentThread();
        WindowClocks.Windows.Changed += (_, _) => Changed?.Invoke();
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
