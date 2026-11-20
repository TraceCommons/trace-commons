using System;
using System.Collections.Generic;
using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// One element's frame clock (AnimationTimeline.swift, GlassAnimationTimeline),
/// fed from WinUI into <see cref="GlassClockCore{TWindow}"/>, which decides:
/// subscribed to the display's frames only while the system lets things
/// move, the element is loaded, it and every element above it are visible,
/// its own window can be seen (<see cref="WindowClocks.Windows"/>, keyed by
/// the element's XamlRoot) and the owner is not paused; drawing at most once
/// per the owner's frame interval. The phase is absolute time, so a clock
/// that stops and starts again picks up where it would have been.
/// </summary>
internal sealed class GlassClock
{
    private readonly FrameworkElement _owner;
    private readonly GlassClockCore<XamlRoot> _core;
    private readonly AncestorVisibility _shown;
    private readonly HeldSubscription _motion;

    public GlassClock(FrameworkElement owner, double interval, Action<double> draw)
    {
        _owner = owner;
        _core = new GlassClockCore<XamlRoot>(
            WindowClocks.Windows,
            interval,
            () =>
            {
                CompositionTarget.Rendering += OnFrame;
                LookTrace.Write($"clock: start {owner.GetType().Name}");
            },
            () =>
            {
                CompositionTarget.Rendering -= OnFrame;
                LookTrace.Write($"clock: stop {owner.GetType().Name} shown={_core!.Shown} window={WindowClocks.Windows.MayTick(owner.XamlRoot)} paused={_core!.Paused}");
            },
            draw,
            () => Now);
        _shown = new AncestorVisibility(owner, shown => _core.Shown = shown);
        _motion = new HeldSubscription(() => GlassMotionSettings.Changed += Update, () => GlassMotionSettings.Changed -= Update);
        owner.Loaded += (_, _) =>
        {
            _motion.Hold();
            _core.Window = owner.XamlRoot;
            _core.AnimationsEnabled = GlassMotionSettings.AnimationsEnabled;
            _shown.Attach();
            _core.Loaded = true;
        };
        owner.Unloaded += (_, _) =>
        {
            _motion.Release();
            _shown.Detach();
            _core.Loaded = false;
        };
    }

    /// <summary>Seconds on one monotonic clock shared by every element.</summary>
    public static double Now => Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

    /// <summary>The owner's own pause (nothing to animate).</summary>
    public bool Paused
    {
        get => _core.Paused;
        set => _core.Paused = value;
    }

    /// <summary>Re-reads every condition; draws the resting frame when the clock stops.</summary>
    public void Update()
    {
        _core.AnimationsEnabled = GlassMotionSettings.AnimationsEnabled;
        _core.Window = _owner.XamlRoot;
        _shown.Recheck();
    }

    private void OnFrame(object? sender, object e) => _core.OnFrame();
}

/// <summary>
/// Whether an element and every element above it are visible
/// (<see cref="EffectiveVisibility"/>), followed live: while attached it
/// listens to each ancestor's Visibility, so a clock inside a tab pane that
/// collapses stops with it. Attached on Loaded, detached on Unloaded; the
/// chain is walked again whenever any of it changes.
/// </summary>
internal sealed class AncestorVisibility
{
    private readonly FrameworkElement _element;
    private readonly Action<bool> _changed;
    private readonly List<(DependencyObject Element, long Token)> _watched = new();
    private bool? _last;

    public AncestorVisibility(FrameworkElement element, Action<bool> changed)
    {
        _element = element;
        _changed = changed;
    }

    public void Attach()
    {
        Detach();
        for (DependencyObject? at = _element; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (at is UIElement ui)
            {
                long token = ui.RegisterPropertyChangedCallback(UIElement.VisibilityProperty, (_, _) => Recheck());
                _watched.Add((ui, token));
            }
        }

        Recheck();
    }

    public void Detach()
    {
        foreach ((DependencyObject element, long token) in _watched)
        {
            element.UnregisterPropertyChangedCallback(UIElement.VisibilityProperty, token);
        }

        _watched.Clear();
        _last = null;
    }

    /// <summary>Walks the chain again and reports a change.</summary>
    public void Recheck()
    {
        bool shown = IsShown(_element);
        if (_last != shown)
        {
            _last = shown;
            _changed(shown);
        }
    }

    /// <summary>Whether <paramref name="element"/> and every ancestor are visible, read once.</summary>
    public static bool IsShown(DependencyObject element)
    {
        var chain = new List<bool>();
        for (DependencyObject? at = element; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (at is UIElement ui)
            {
                chain.Add(ui.Visibility == Visibility.Visible);
            }
        }

        return EffectiveVisibility.Shown(chain);
    }
}
