using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// One credit-arrival celebration on the tray and the taskbar, planned from
/// the Mac's rules (<c>DockCreditAnimator.celebrate</c>,
/// TraceCommonsApp/DockCreditAnimator.swift:93-130, and
/// <c>MenuBarStripLabel</c>'s task, Views/Monitor/MenuBarGlassPanel.swift:689-711),
/// so the window only plays what this decides. Owner decision W12-D4: the
/// tray's gleam, hop and mini coins, the taskbar's attention flash, and the
/// taskbar's coin burst, all behind the one Settings switch.
/// </summary>
public sealed record CreditCelebrationPlan(
    bool Play,
    bool FlashTaskbar,
    IReadOnlyList<double> FrameTimes,
    TimeSpan FrameWait)
{
    public static readonly CreditCelebrationPlan Nothing = new(false, false, Array.Empty<double>(), TimeSpan.Zero);
}

public static class CreditCelebration
{
    /// <summary>
    /// The plan for one arrival. Switch off: nothing, and anything playing
    /// stops and the icons are restored (DockCreditAnimator.swift:94-97;
    /// GeneralSection.swift:42). Reduce motion: the one still frame at 0.45 s,
    /// held 1.6 s, and no attention request (:98-101). The flash is the Mac's
    /// single informational bounce, which the Dock gives only to an app in
    /// the background: so only while the window is not the active one, and
    /// only when there is a taskbar button to flash.
    /// </summary>
    public static CreditCelebrationPlan Plan(bool switchOn, bool reduceMotion, bool windowActive, bool hasTaskbarButton)
    {
        if (!switchOn)
        {
            return CreditCelebrationPlan.Nothing;
        }

        return new CreditCelebrationPlan(
            Play: true,
            FlashTaskbar: !reduceMotion && !windowActive && hasTaskbarButton,
            FrameTimes: BalanceArrival.IconAnimation.FrameTimes(reduceMotion),
            FrameWait: reduceMotion ? BalanceArrival.IconAnimation.ReduceMotionHold : BalanceArrival.IconAnimation.FrameInterval);
    }

    /// <summary>
    /// The Settings switch's stored value: on unless the person turned it off
    /// (<c>BalanceArrival.IconAnimation.isEnabled</c>, BalanceArrival.swift:142-145).
    /// </summary>
    public static bool IsOn(bool? stored) => stored ?? true;

    /// <summary>
    /// Plays a plan's frames: the frame to draw at each step, then null once
    /// it is over and the icons go back to rest. A clock-free player the
    /// window ticks, so a new arrival restarts it from the start and the
    /// switch going off stops it at once.
    /// </summary>
    public sealed class Player
    {
        private IReadOnlyList<double> _frames = Array.Empty<double>();
        private int _next;

        /// <summary>Whether frames remain.</summary>
        public bool Playing => _next < _frames.Count;

        /// <summary>The wait between frames for the plan being played.</summary>
        public TimeSpan Wait { get; private set; }

        /// <summary>Starts <paramref name="plan"/> from its first frame, replacing anything under way.</summary>
        public void Start(CreditCelebrationPlan plan)
        {
            _frames = plan.Play ? plan.FrameTimes : Array.Empty<double>();
            Wait = plan.FrameWait;
            _next = 0;
        }

        /// <summary>Stops at once: the next <see cref="Step"/> is the rest frame.</summary>
        public void Stop() => _next = _frames.Count;

        /// <summary>The next frame's elapsed time, or null for rest (and the end of the run).</summary>
        public double? Step() => _next < _frames.Count ? _frames[_next++] : null;
    }
}
