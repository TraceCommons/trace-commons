using System;
using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// One element's frame clock (AnimationTimeline.swift, GlassAnimationTimeline):
/// subscribed to the display's frames only while <see cref="GlassFrameGate"/>
/// says it may run (animations on, the element loaded and shown, the window
/// visible, the owner not paused), and drawing at most once per the owner's
/// frame interval. The phase is absolute time, so a clock that stops and
/// starts again picks up where it would have been. The owner stops it on
/// Unloaded by construction: the gate's Loaded goes false there.
/// </summary>
internal sealed class GlassClock
{
    private readonly FrameworkElement _owner;
    private readonly GlassFrameGate _gate;
    private readonly Action<double> _draw;
    private bool _subscribed;

    public GlassClock(FrameworkElement owner, double interval, Action<double> draw)
    {
        _owner = owner;
        _gate = new GlassFrameGate(interval);
        _draw = draw;
        owner.Loaded += (_, _) =>
        {
            _gate.Loaded = true;
            GlassMotionSettings.Changed += Update;
            Update();
        };
        owner.Unloaded += (_, _) =>
        {
            _gate.Loaded = false;
            GlassMotionSettings.Changed -= Update;
            Update();
        };
        owner.RegisterPropertyChangedCallback(UIElement.VisibilityProperty, (_, _) => Update());
    }

    /// <summary>Seconds on one monotonic clock shared by every element.</summary>
    public static double Now => Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

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

    /// <summary>Re-reads every condition; draws the resting frame when the clock stops.</summary>
    public void Update()
    {
        _gate.AnimationsEnabled = GlassMotionSettings.AnimationsEnabled;
        _gate.WindowVisible = WindowClocks.Running && _owner.Visibility == Visibility.Visible;
        bool run = _gate.Running;
        if (run && !_subscribed)
        {
            _gate.Reset();
            CompositionTarget.Rendering += OnFrame;
            _subscribed = true;
        }
        else if (!run && _subscribed)
        {
            CompositionTarget.Rendering -= OnFrame;
            _subscribed = false;
        }

        if (!run)
        {
            // Stopped: the state as it stands, or the still frame under
            // reduce motion, is drawn once.
            _draw(Now);
        }
    }

    private void OnFrame(object? sender, object e)
    {
        double now = Now;
        if (_gate.ShouldDraw(now))
        {
            _draw(now);
        }
    }
}
