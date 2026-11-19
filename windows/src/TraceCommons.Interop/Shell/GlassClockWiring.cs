using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.CompilerServices;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// Whether each window's clocks may run (AnimationTimeline.swift:4-32: a
/// timeline pauses unless its own window is visible). Every window reports
/// its own <see cref="WindowClockGate"/> here under its own key, so a clock
/// asks about the window it is drawn in: the Settings window keeps moving
/// while the main window sits in the tray, and a minimised Settings window
/// stops its own clocks while the main window's run on. A window that never
/// reported may tick; an element in no window may not; a closed window stays
/// stopped. Keys are held weakly, so a closed window's tree is not kept alive.
/// </summary>
public sealed class WindowClockRegistry<TWindow> where TWindow : class
{
    private readonly ConditionalWeakTable<TWindow, StrongBox<bool>> _mayTick = new();

    /// <summary>Raised with the window and its new answer whenever <see cref="MayTick"/> changes for it.</summary>
    public event Action<TWindow, bool>? Changed;

    /// <summary>Whether clocks drawn in <paramref name="window"/> may run.</summary>
    public bool MayTick(TWindow? window) =>
        window is not null && (!_mayTick.TryGetValue(window, out StrongBox<bool>? mayTick) || mayTick.Value);

    /// <summary>Records <paramref name="window"/>'s gate.</summary>
    public void Set(TWindow window, bool mayTick)
    {
        bool before = MayTick(window);
        _mayTick.GetOrCreateValue(window).Value = mayTick;
        if (before != mayTick)
        {
            Changed?.Invoke(window, mayTick);
        }
    }
}

/// <summary>
/// Whether an element is drawn at all: it and every element above it are
/// visible. A clock inside a collapsed tab pane is not seen however its own
/// visibility reads.
/// </summary>
public static class EffectiveVisibility
{
    /// <param name="selfAndAncestorsVisible">The element's own visibility, then each ancestor's up to the root.</param>
    public static bool Shown(IEnumerable<bool> selfAndAncestorsVisible) => selfAndAncestorsVisible.All(visible => visible);
}

/// <summary>
/// One element's clock, as the app wires it (AnimationTimeline.swift,
/// <c>GlassAnimationTimeline</c>): the element's facts go in (loaded, shown
/// with its ancestors, its window, the system's animation switch, its own
/// pause) and the clock subscribes to the display's frames only while all
/// allow it, draws at most once per its interval, and draws its resting
/// frame once whenever it stops. The app's <c>GlassClock</c> feeds it from
/// WinUI; nothing here knows WinUI, so the wiring is tested with a fake
/// frame source.
/// </summary>
public sealed class GlassClockCore<TWindow> where TWindow : class
{
    private readonly WindowClockRegistry<TWindow> _windows;
    private readonly GlassFrameGate _gate;
    private readonly Action _subscribe;
    private readonly Action _unsubscribe;
    private readonly Action<double> _draw;
    private readonly Func<double> _now;
    private TWindow? _window;
    private bool _shown = true;
    private bool _listening;

    public GlassClockCore(
        WindowClockRegistry<TWindow> windows,
        double interval,
        Action subscribe,
        Action unsubscribe,
        Action<double> draw,
        Func<double> now)
    {
        _windows = windows;
        _gate = new GlassFrameGate(interval);
        _subscribe = subscribe;
        _unsubscribe = unsubscribe;
        _draw = draw;
        _now = now;
    }

    /// <summary>Whether the clock is subscribed to the display's frames.</summary>
    public bool Subscribed { get; private set; }

    /// <summary>The window the element is drawn in.</summary>
    public TWindow? Window
    {
        get => _window;
        set
        {
            _window = value;
            Update();
        }
    }

    /// <summary>The element is in the tree. Its window's gate is listened to only while it is.</summary>
    public bool Loaded
    {
        get => _gate.Loaded;
        set
        {
            _gate.Loaded = value;
            if (value && !_listening)
            {
                _windows.Changed += OnWindowChanged;
                _listening = true;
            }
            else if (!value && _listening)
            {
                _windows.Changed -= OnWindowChanged;
                _listening = false;
            }

            Update();
        }
    }

    /// <summary>The element and every ancestor are visible (<see cref="EffectiveVisibility"/>).</summary>
    public bool Shown
    {
        get => _shown;
        set
        {
            _shown = value;
            Update();
        }
    }

    /// <summary>The system lets things move.</summary>
    public bool AnimationsEnabled
    {
        get => _gate.AnimationsEnabled;
        set
        {
            _gate.AnimationsEnabled = value;
            Update();
        }
    }

    /// <summary>The owner's own pause (nothing to animate).</summary>
    public bool Paused
    {
        get => _gate.Paused;
        set
        {
            _gate.Paused = value;
            Update();
        }
    }

    /// <summary>Whether a one-shot animation of this element may run now: the same test less the pause.</summary>
    public bool MayMove => _gate.AnimationsEnabled && _gate.Loaded && _shown && _windows.MayTick(_window);

    /// <summary>Re-reads every condition; subscribes or unsubscribes, and draws the resting frame when stopped.</summary>
    public void Update()
    {
        _gate.WindowVisible = _shown && _windows.MayTick(_window);
        bool run = _gate.Running;
        if (run && !Subscribed)
        {
            _gate.Reset();
            _subscribe();
            Subscribed = true;
        }
        else if (!run && Subscribed)
        {
            _unsubscribe();
            Subscribed = false;
        }

        if (!run)
        {
            _draw(_now());
        }
    }

    /// <summary>One display frame: draws it if the gate allows.</summary>
    public void OnFrame()
    {
        double now = _now();
        if (Subscribed && _gate.ShouldDraw(now))
        {
            _draw(now);
        }
    }

    private void OnWindowChanged(TWindow window, bool mayTick)
    {
        if (ReferenceEquals(window, _window))
        {
            Update();
        }
    }
}

/// <summary>
/// Which run of each animated property is current. A run that is cut short
/// by the next run (or a settle) of the same property is stale: when its
/// completion arrives afterwards it must not write its end value over the
/// run that replaced it.
/// </summary>
public sealed class MotionRuns<TProperty> where TProperty : notnull
{
    private readonly Dictionary<TProperty, int> _current = new();

    /// <summary>Starts a new run of <paramref name="property"/> and returns its number.</summary>
    public int Begin(TProperty property)
    {
        int next = _current.TryGetValue(property, out int run) ? run + 1 : 1;
        _current[property] = next;
        return next;
    }

    /// <summary>Whether <paramref name="run"/> is still the latest run of <paramref name="property"/>.</summary>
    public bool IsCurrent(TProperty property, int run) =>
        _current.TryGetValue(property, out int current) && current == run;
}
