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
    private readonly ConditionalWeakTable<TWindow, StrongBox<bool>> _focused = new();

    /// <summary>
    /// Raised with the window and its <see cref="MayTick"/> answer whenever
    /// that answer or the window's focus changes, so every clock re-reads
    /// its rate.
    /// </summary>
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

    /// <summary>Whether <paramref name="window"/> is the active window. One that never reported counts as focused.</summary>
    public bool Focused(TWindow? window) =>
        window is not null && (!_focused.TryGetValue(window, out StrongBox<bool>? focused) || focused.Value);

    /// <summary>Records whether <paramref name="window"/> is the active window (its Activated and Deactivated).</summary>
    public void SetFocused(TWindow window, bool focused)
    {
        bool before = Focused(window);
        _focused.GetOrCreateValue(window).Value = focused;
        if (before != focused)
        {
            Changed?.Invoke(window, MayTick(window));
        }
    }

    /// <summary>How much attention <paramref name="window"/> has: hidden, visible but not focused, or focused.</summary>
    public WindowAttention Attention(TWindow? window) =>
        !MayTick(window) ? WindowAttention.Hidden : Focused(window) ? WindowAttention.Focused : WindowAttention.Unfocused;

    /// <summary>The interval a clock in <paramref name="window"/> may run at (<see cref="FocusFrameRule"/>), or null: stopped.</summary>
    public double? Interval(TWindow? window, double wanted) => FocusFrameRule.Interval(wanted, Attention(window));
}

/// <summary>Where a window stands for its animations.</summary>
public enum WindowAttention
{
    /// <summary>Hidden to the tray, minimised, closed, or not in a window: nothing moves.</summary>
    Hidden,

    /// <summary>On screen, but another window is active.</summary>
    Unfocused,

    /// <summary>The active window.</summary>
    Focused,
}

/// <summary>
/// The one rule every animated surface's clock follows (owner, 2026-11-20):
/// full rate while its window is focused, at most 15 frames a second while
/// the window is visible but another is active, and stopped while it is
/// hidden or minimised. A window that becomes focused again runs at its
/// full rate from its next frame. Reduce Motion is separate and unchanged.
/// </summary>
public static class FocusFrameRule
{
    /// <summary>The shortest interval an unfocused window's clocks may draw at: 15 frames a second.</summary>
    public const double UnfocusedInterval = 1.0 / 15;

    /// <summary>
    /// The interval to draw at, given the one the surface wants
    /// (<paramref name="wanted"/> seconds; 0 is every display frame), or
    /// null when nothing may draw.
    /// </summary>
    public static double? Interval(double wanted, WindowAttention attention) => attention switch
    {
        WindowAttention.Hidden => null,
        WindowAttention.Unfocused => Math.Max(wanted, UnfocusedInterval),
        _ => wanted,
    };
}

/// <summary>
/// A frame gate for a surface that draws on the display's frames
/// (CompositionTarget.Rendering): asks the window's <see cref="FocusFrameRule"/>
/// interval on every frame, so focus changes take effect on the next one.
/// </summary>
public sealed class FocusFramePace<TWindow> where TWindow : class
{
    private readonly WindowClockRegistry<TWindow> _windows;
    private double? _last;

    public FocusFramePace(WindowClockRegistry<TWindow> windows) => _windows = windows;

    /// <summary>
    /// Whether to draw the frame at <paramref name="now"/> seconds in
    /// <paramref name="window"/>, for a surface that wants
    /// <paramref name="wanted"/> seconds between frames (0: every frame). A
    /// frame comes no sooner than the interval after the last one drawn,
    /// less a quarter of a 60 Hz frame for jitter.
    /// </summary>
    public bool Due(TWindow? window, double now, double wanted = 0)
    {
        if (_windows.Interval(window, wanted) is not double interval)
        {
            return false;
        }

        if (_last is double last && now - last < interval - 1.0 / 240 && now >= last)
        {
            return false;
        }

        _last = now;
        return true;
    }

    /// <summary>
    /// Whether a surface whose window may tick should leave out the frame at
    /// <paramref name="now"/>: true only while its window is visible and the
    /// frame is not due. While the window is hidden this is false, so the
    /// surface's own stop logic still runs.
    /// </summary>
    public bool Skips(TWindow? window, double now, double wanted = 0) =>
        _windows.MayTick(window) && !Due(window, now, wanted);
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
        _gate.Floor = _windows.Focused(_window) ? 0 : FocusFrameRule.UnfocusedInterval;
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

/// <summary>
/// A handler an element keeps on a long-lived event between Loaded and
/// Unloaded, held at most once. WinUI can raise Loaded twice without an
/// Unloaded between; a bare <c>+=</c> on each Loaded then adds the handler
/// twice, the one Unloaded removes one copy, and a static event keeps the
/// element alive for the life of the process.
/// </summary>
public sealed class HeldSubscription
{
    private readonly Action _add;
    private readonly Action _remove;

    public HeldSubscription(Action add, Action remove)
    {
        _add = add;
        _remove = remove;
    }

    /// <summary>Whether the handler is on the event.</summary>
    public bool Held { get; private set; }

    /// <summary>Adds the handler unless it is already held.</summary>
    public void Hold()
    {
        if (Held)
        {
            return;
        }

        _add();
        Held = true;
    }

    /// <summary>Removes the handler if it is held.</summary>
    public void Release()
    {
        if (!Held)
        {
            return;
        }

        _remove();
        Held = false;
    }
}
