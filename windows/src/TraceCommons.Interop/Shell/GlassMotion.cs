using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The Mac's motion (TCDesign/Styling/GlassMotion.swift) as numbers: the one
/// token curve, SwiftUI's ease-out, and its two spring forms. Every animation
/// in the Windows app takes its curve, duration or spring from here or from
/// <see cref="GlassControlMotion"/>, never from WinUI's own timings, and none
/// runs while the system's animations are off (GlassMotion.swift:21-35: nil
/// under Reduce Motion).
/// </summary>
public static class GlassMotion
{
    /// <summary>
    /// The token curve, <c>timingCurve(0.2, 0.8, 0.2, 1)</c>
    /// (GlassMotion.swift:7-11), at <paramref name="t"/> in [0, 1].
    /// </summary>
    public static double Curve(double t) => MonitorMotion.CubicBezier(
        GlassTokens.Motion.EaseX1, GlassTokens.Motion.EaseY1, GlassTokens.Motion.EaseX2, GlassTokens.Motion.EaseY2, t);

    /// <summary>SwiftUI's <c>.easeOut</c>: a cubic Bezier through (0, 0) and (0.58, 1).</summary>
    public const double EaseOutX1 = 0, EaseOutY1 = 0, EaseOutX2 = 0.58, EaseOutY2 = 1;

    /// <summary>SwiftUI's <c>.easeOut</c> at <paramref name="t"/> in [0, 1].</summary>
    public static double EaseOut(double t) => MonitorMotion.CubicBezier(EaseOutX1, EaseOutY1, EaseOutX2, EaseOutY2, t);

    /// <summary>
    /// <c>.spring(response:dampingFraction:)</c> from 0 towards 1,
    /// <paramref name="seconds"/> in.
    /// </summary>
    public static double Spring(double response, double damping, double seconds) =>
        MonitorMotion.Spring(response, damping, seconds);

    /// <summary>
    /// <c>.spring(duration:bounce:)</c>: SwiftUI's response is the duration
    /// and its damping one less the bounce (read from SwiftUI's own
    /// <c>Spring(duration: 0.9, bounce: 0.04)</c>: response 0.9, damping 0.96).
    /// </summary>
    public static double SpringDuration(double duration, double bounce, double seconds) =>
        MonitorMotion.Spring(duration, 1 - bounce, seconds);

    /// <summary>The first run's spatial spring (GlassMotion.swift:39-41): 0.9 s, bounce 0.04.</summary>
    public static double Spatial(double seconds, double duration = 0.9) => SpringDuration(duration, 0.04, seconds);

    /// <summary>The expressive spring (GlassMotion.swift:46-48): 0.78 s, bounce 0.32.</summary>
    public static double Expressive(double seconds, double duration = 0.78) => SpringDuration(duration, 0.32, seconds);

    /// <summary>
    /// How long a spring is drawn before it is set to its end: the first time
    /// after which it stays within <paramref name="tolerance"/> of 1. Searched
    /// at 1 ms over at most <paramref name="limit"/> seconds.
    /// </summary>
    public static double SettleTime(Func<double, double> spring, double tolerance = 0.001, double limit = 3)
    {
        double settled = 0;
        for (double t = 0; t <= limit; t += 0.001)
        {
            if (Math.Abs(spring(t) - 1) > tolerance)
            {
                settled = t + 0.001;
            }
        }

        return Math.Min(settled, limit);
    }
}

/// <summary>A curve an animation runs on.</summary>
public enum MotionCurve
{
    /// <summary>The token curve, (0.2, 0.8, 0.2, 1).</summary>
    Token,
    /// <summary>SwiftUI's ease-out, (0, 0, 0.58, 1).</summary>
    EaseOut,
    /// <summary>A spring, <c>.spring(response:dampingFraction:)</c>.</summary>
    Spring,
}

/// <summary>
/// One animation's timing: its curve and duration, or its spring. Duration
/// is in seconds; for a spring it is the time to settle, and
/// <see cref="Response"/> and <see cref="Damping"/> carry the spring.
/// </summary>
public readonly record struct MotionSpec(MotionCurve Curve, double Duration, double Response = 0, double Damping = 0)
{
    public static MotionSpec Token(double duration) => new(MotionCurve.Token, duration);

    public static MotionSpec EaseOutFor(double duration) => new(MotionCurve.EaseOut, duration);

    public static MotionSpec SpringOf(double response, double damping) =>
        new(MotionCurve.Spring, GlassMotion.SettleTime(t => GlassMotion.Spring(response, damping, t)), response, damping);

    /// <summary>Progress from 0 to 1 (a spring may overshoot) <paramref name="seconds"/> in.</summary>
    public double At(double seconds) => Curve switch
    {
        MotionCurve.Token => GlassMotion.Curve(Math.Clamp(seconds / Duration, 0, 1)),
        MotionCurve.EaseOut => GlassMotion.EaseOut(Math.Clamp(seconds / Duration, 0, 1)),
        _ => seconds >= Duration ? 1 : GlassMotion.Spring(Response, Damping, seconds),
    };

    /// <summary>
    /// Key frames of the progress, one per display frame at
    /// <paramref name="perSecond"/>, for a compositor animation to run
    /// between: the compositor then draws at the display's rate, which is
    /// the rate SwiftUI animates at.
    /// </summary>
    public IReadOnlyList<(double Fraction, double Value)> KeyFrames(double perSecond = 60) =>
        MonitorMotion.KeyFrames(At, Duration, perSecond);
}

/// <summary>
/// The motion of the stock controls the Windows app re-templates to the
/// Mac's shapes (spec W5-D3 (a)), one row of the spec's "Animation
/// coverage" table each, with the Mac's numbers.
/// </summary>
public static class GlassControlMotion
{
    /// <summary>Hover fill fade: opacity 0 to 1, fast on the token curve (GlassHover.swift:40).</summary>
    public static readonly MotionSpec HoverFade = MotionSpec.Token(GlassTokens.Motion.Fast);

    /// <summary>
    /// Press: a fill 8% darker, or a black wash of 0.08 under a label with
    /// no fill, fast on the token curve (GlassPress.swift:14, :20-27, :107-127).
    /// </summary>
    public static readonly MotionSpec Press = MotionSpec.Token(GlassTokens.Motion.Fast);

    /// <summary>GlassPress.darkening (GlassPress.swift:14).</summary>
    public const double PressDarkening = 0.08;

    /// <summary>
    /// A pressed fill (GlassPress.brightness, applied as SwiftUI's
    /// <c>.brightness(-0.08)</c>): each sRGB channel 0.08 lower, held at 0.
    /// </summary>
    public static (byte R, byte G, byte B) Pressed(byte r, byte g, byte b)
    {
        static byte Down(byte c) => (byte)Math.Clamp((int)Math.Round(c - PressDarkening * 255), 0, 255);
        return (Down(r), Down(g), Down(b));
    }

    /// <summary>
    /// The switch knob's slide: ease-out over standard (0.22 s) for a toggle
    /// and a Settings switch, fast (0.15 s) for the watch switch
    /// (Controls.swift:1144-1146, :1167).
    /// </summary>
    public static MotionSpec SwitchKnob(GlassSwitchKind kind) =>
        MotionSpec.EaseOutFor(kind == GlassSwitchKind.Watch ? GlassTokens.Motion.Fast : GlassTokens.Motion.Standard);

    /// <summary>
    /// The switch's track and knob, as the Mac draws them
    /// (Controls.swift:1150-1177): its size, the knob's 18 across, and the
    /// knob's inset from the track's end.
    /// </summary>
    public static (double Width, double Height, double Knob, double Inset) SwitchGeometry(GlassSwitchKind kind) =>
        kind == GlassSwitchKind.Watch
            ? (GlassTokens.Size.WatchSwitchWidth, GlassTokens.Size.WatchSwitchHeight, 18, 2)
            : (GlassTokens.Size.ToggleWidth, GlassTokens.Size.ToggleHeight, 18, 3);

    /// <summary>The knob's left edge within the track, on or off.</summary>
    public static double KnobOffset(GlassSwitchKind kind, bool isOn)
    {
        (double width, _, double knob, double inset) = SwitchGeometry(kind);
        return isOn ? width - inset - knob : inset;
    }

    /// <summary>
    /// The expander's chevron: 0 to 90 degrees, fast on the token curve
    /// (Controls.swift:645-652, GlassExpander; PopoverNotice.swift:110).
    /// </summary>
    public static readonly MotionSpec ExpanderChevron = MotionSpec.Token(GlassTokens.Motion.Fast);

    /// <summary>The expander chevron's angle, open or closed.</summary>
    public static double ExpanderAngle(bool isOpen) => isOpen ? 90 : 0;

    /// <summary>
    /// Whether a chevron turns on an animation. Only GlassExpander's does:
    /// the split button's (Controls.swift:1019), the tree row's
    /// (ListRow.swift:150) and the Projects list's (ProjectsSection.swift:428)
    /// change with no animation, and snap on Windows too.
    /// </summary>
    public static bool ChevronAnimates(GlassChevron chevron) => chevron == GlassChevron.Expander;

    /// <summary>The modal's layer fade: standard on the token curve (Modal.swift:491-495).</summary>
    public static readonly MotionSpec ModalFade = MotionSpec.Token(GlassTokens.Motion.Standard);

    /// <summary>The tab ember's glide between tabs: spring(0.35, 0.85) (Navigation.swift:266).</summary>
    public static readonly MotionSpec TabEmberGlide = MotionSpec.SpringOf(0.35, 0.85);

    /// <summary>A movable tab lifting under the pointer: spring(0.25, 0.8) (Navigation.swift:247).</summary>
    public static readonly MotionSpec TabLift = MotionSpec.SpringOf(0.25, 0.8);

    /// <summary>A dragged tab dropping into its slot: spring(0.3, 0.85) (Navigation.swift:416).</summary>
    public static readonly MotionSpec TabDrop = MotionSpec.SpringOf(0.3, 0.85);
}

/// <summary>A Mac switch's kind (Controls.swift:1105-1107).</summary>
public enum GlassSwitchKind
{
    Standard,
    Settings,
    Watch,
}

/// <summary>The Mac's chevrons, by where they are drawn.</summary>
public enum GlassChevron
{
    /// <summary>GlassExpander (Controls.swift:637-657): turns on the fast curve.</summary>
    Expander,
    /// <summary>The split button's menu chevron (Controls.swift:1019): snaps.</summary>
    SplitButton,
    /// <summary>A tree row's disclosure (ListRow.swift:150): snaps.</summary>
    TreeRow,
    /// <summary>A Settings project row (ProjectsSection.swift:428): snaps.</summary>
    Projects,
}

/// <summary>
/// GlassSpinner (Feedback.swift:15-63): a 12 across ring whose quarter arc
/// turns once every <c>motion.spin</c>, at most 60 frames a second, still
/// under reduce motion.
/// </summary>
public static class GlassSpinnerMotion
{
    /// <summary>Feedback.swift:34, <c>frameInterval</c>.</summary>
    public const double FrameInterval = 1.0 / 60;

    /// <summary>The arc's share of the ring (Feedback.swift:53, <c>trim(from: 0, to: 0.25)</c>).</summary>
    public const double Arc = 0.25;

    /// <summary>The track's ink alpha (Feedback.swift:51, <c>GlassColor.ink(0.18)</c>).</summary>
    public const double TrackAlpha = 0.18;

    /// <summary>The arc's turn in degrees at <paramref name="time"/> seconds (Feedback.swift:38-42).</summary>
    public static double Angle(double time, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return 0;
        }

        double period = GlassTokens.Motion.Spin;
        return time % period / period * 360;
    }
}

/// <summary>
/// GlassSkeleton (Feedback.swift:74-107): opacity breathing between 1 and
/// <c>opacity.skeletonDim</c>, half a <c>motion.pulse</c> each way on the
/// token curve, repeating and reversing; still at 1 under reduce motion.
/// </summary>
public static class GlassSkeletonMotion
{
    /// <summary>One leg of the pulse, seconds (Feedback.swift:88-90).</summary>
    public static double Leg => GlassTokens.Motion.Pulse / 2;

    /// <summary>The bar's opacity <paramref name="time"/> seconds after it appeared.</summary>
    public static double Opacity(double time, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return 1;
        }

        double dim = GlassTokens.Opacity.SkeletonDim;
        double leg = Leg;
        double phase = time % (2 * leg);
        // Out on the curve, then the same leg played backwards
        // (repeatForever(autoreverses: true)).
        double progress = phase < leg ? GlassMotion.Curve(phase / leg) : GlassMotion.Curve(1 - (phase - leg) / leg);
        return 1 + (dim - 1) * progress;
    }
}

/// <summary>
/// GlassRefreshButton (Containers.swift:71-143): the icon turns once per
/// <c>motion.spin</c> at 30 frames a second; a click completes one full turn
/// even when the answer is quick; it turns for as long as the refresh runs;
/// still under reduce motion.
/// </summary>
public static class GlassRefreshMotion
{
    /// <summary>Containers.swift:112, <c>minimumInterval: 1 / 30</c>.</summary>
    public const double FrameInterval = 1.0 / 30;

    /// <summary>Containers.swift:101-105.</summary>
    public static double Angle(double elapsed, bool isRefreshing, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return 0;
        }

        double turns = Math.Max(0, elapsed) / GlassTokens.Motion.Spin;
        return (isRefreshing ? turns % 1 : Math.Min(turns, 1)) * 360;
    }

    /// <summary>Containers.swift:107-109: it ticks only while refreshing or finishing a click's turn.</summary>
    public static bool Animates(bool isRefreshing, bool completingClick, bool reduceMotion) =>
        !reduceMotion && (isRefreshing || completingClick);
}

/// <summary>The Mac's rule tabs (Navigation.swift:200-420): ember, halo, lift and drag.</summary>
public static class GlassTabMotion
{
    /// <summary>How far the ember line reaches past its tab, each side, as a share of its width (Navigation.swift:225).</summary>
    public const double EmberReach = 0.3;

    /// <summary>The ember line's thickness (Navigation.swift:227).</summary>
    public const double EmberLine = 2;

    /// <summary>The halo's height (Navigation.swift:228).</summary>
    public const double HaloHeight = 26;

    /// <summary>A lifted tab's scale, rise and shadow (Navigation.swift:360-362, :296).</summary>
    public const double LiftScale = 1.06, LiftRise = 2, LiftShadowOpacity = 0.18, LiftShadowRadius = 8, LiftShadowY = 3;

    /// <summary>How far a lifted tab travels before letting go moves it (Navigation.swift:364).</summary>
    public const double DragThreshold = 6;

    /// <summary>Navigation.swift:369-373: the slot a dragged tab lands in.</summary>
    public static int Landing(int from, double offset, double tabWidth, int lowest, int highest)
    {
        if (tabWidth <= 0)
        {
            return from;
        }

        int slot = from + (int)Math.Round(offset / tabWidth, MidpointRounding.AwayFromZero);
        return Math.Min(Math.Max(slot, lowest), highest);
    }

    /// <summary>Navigation.swift:377-379.</summary>
    public static bool HasMoved(double translation) => Math.Abs(translation) >= DragThreshold;

    /// <summary>
    /// The ember line's extent under a tab at <paramref name="tabX"/> wide
    /// <paramref name="tabWidth"/>: it reaches 30% past each side.
    /// </summary>
    public static (double X, double Width) Ember(double tabX, double tabWidth) =>
        (tabX - tabWidth * EmberReach, tabWidth * (1 + 2 * EmberReach));

    /// <summary>
    /// The halo's frame: 1.1 of the tab's width, centred on it, twice the
    /// halo height tall centred on the rule (Navigation.swift:319-327: an
    /// elliptical gradient from the foot's centre to half the frame).
    /// </summary>
    public static (double X, double Width, double Height) Halo(double tabX, double tabWidth) =>
        (tabX - tabWidth * 0.05, tabWidth * 1.1, HaloHeight * 2);

    /// <summary>
    /// The halo's alpha at a point <paramref name="dx"/>, <paramref name="dy"/>
    /// from the rule under the tab's centre, as a share of its full alpha:
    /// an elliptical gradient from 1 at the centre to 0 at half the frame
    /// (EllipticalGradient, startRadiusFraction 0, endRadiusFraction 0.5).
    /// </summary>
    public static double HaloShare(double dx, double dy, double tabWidth)
    {
        double rx = tabWidth * 1.1 / 2, ry = HaloHeight;
        double r = Math.Sqrt(dx * dx / (rx * rx) + dy * dy / (ry * ry));
        return Math.Clamp(1 - r, 0, 1);
    }

    /// <summary>
    /// The ember line's alpha at <paramref name="dx"/> from the tab's centre,
    /// as a share of its full alpha: a linear gradient clear, full, clear
    /// across its reach.
    /// </summary>
    public static double EmberShare(double dx, double tabWidth)
    {
        double half = tabWidth * (1 + 2 * EmberReach) / 2;
        return Math.Clamp(1 - Math.Abs(dx) / half, 0, 1);
    }
}

/// <summary>
/// When an animation may draw a frame (AnimationTimeline.swift:4-32,
/// <c>GlassAnimationTimeline</c>): only while the system lets things move,
/// its element is loaded, and its window can be seen (shown, not minimised,
/// not hidden to the tray); and at most once per its frame interval. The
/// phase comes from absolute time, so a clock that stops and starts again
/// picks up where it would have been.
/// </summary>
public sealed class GlassFrameGate
{
    private readonly double _interval;
    private double? _lastDrawn;

    /// <param name="interval">The minimum seconds between frames; 0 draws every display frame.</param>
    public GlassFrameGate(double interval) => _interval = interval;

    public bool AnimationsEnabled { get; set; } = true;

    public bool Loaded { get; set; }

    public bool WindowVisible { get; set; } = true;

    /// <summary>The caller's own pause (GlassAnimationTimeline's <c>paused</c>).</summary>
    public bool Paused { get; set; }

    /// <summary>Whether the clock should be subscribed at all.</summary>
    public bool Running => AnimationsEnabled && Loaded && WindowVisible && !Paused;

    /// <summary>
    /// Called on each display frame at <paramref name="now"/> seconds:
    /// whether to draw this one. A frame comes no sooner than the interval
    /// after the last drawn, less a quarter of a 60 Hz frame for jitter.
    /// </summary>
    public bool ShouldDraw(double now)
    {
        if (!Running)
        {
            return false;
        }

        if (_lastDrawn is double last && now - last < _interval - 1.0 / 240)
        {
            return false;
        }

        _lastDrawn = now;
        return true;
    }

    /// <summary>Forgets the last frame, so the next one draws at once when the clock starts again.</summary>
    public void Reset() => _lastDrawn = null;
}

/// <summary>
/// The tab ember moving between tabs (Navigation.swift:288,
/// <c>matchedGeometryEffect</c> under spring(0.35, 0.85) at :266): the ember
/// takes its new tab's frame at once, and its visual starts where the old
/// one was, offset and scaled about its centre, and springs to rest.
/// </summary>
public static class TabEmberGlide
{
    /// <summary>
    /// The start of the glide: the translation along the strip and the
    /// horizontal scale that put the new tab's ember over the old tab's.
    /// </summary>
    public static (double TranslateX, double ScaleX) From(double oldX, double oldWidth, double newX, double newWidth)
    {
        if (newWidth <= 0 || oldWidth <= 0)
        {
            return (0, 1);
        }

        double oldCentre = oldX + oldWidth / 2, newCentre = newX + newWidth / 2;
        return (oldCentre - newCentre, oldWidth / newWidth);
    }

    /// <summary>
    /// What a call to put the ember under the tab shown does. The Mac
    /// animates the ember only when the selection changes
    /// (<c>.animation(_, value: selection)</c>, Navigation.swift:266): a
    /// first placement or a relayout under the same tab moves it at once, and
    /// the same tab at the same frame leaves it alone, so a glide under way
    /// is never cut short by a refresh that changed nothing.
    /// </summary>
    /// <param name="placed">The ember has been placed before.</param>
    /// <param name="sameTab">The tab shown is the one it was placed under.</param>
    /// <param name="sameFrame">That tab's frame in the strip has not changed.</param>
    public static EmberMove Decide(bool placed, bool sameTab, bool sameFrame) =>
        !placed ? EmberMove.Place
        : !sameTab ? EmberMove.Glide
        : sameFrame ? EmberMove.Stay
        : EmberMove.Place;

    /// <summary>
    /// The start of a glide that begins while another is under way: from
    /// where the ember is drawn now, not from the tab it was gliding to
    /// (SwiftUI's matchedGeometryEffect retargets from the current
    /// presentation). <paramref name="restX"/> and <paramref name="restWidth"/>
    /// are the frame the running glide rests on, <paramref name="translateX"/>
    /// and <paramref name="scaleX"/> where it started, <paramref name="seconds"/>
    /// how long it has run.
    /// </summary>
    public static (double TranslateX, double ScaleX) Retarget(
        double restX, double restWidth, double translateX, double scaleX, double seconds, double newX, double newWidth)
    {
        (double tx, double sx) = At(translateX, scaleX, seconds);
        double drawnWidth = restWidth * sx;
        double drawnX = restX + restWidth / 2 + tx - drawnWidth / 2;
        return From(drawnX, drawnWidth, newX, newWidth);
    }

    /// <summary>The glide <paramref name="seconds"/> in: translation and scale, on the Mac's spring.</summary>
    public static (double TranslateX, double ScaleX) At(double translateX, double scaleX, double seconds)
    {
        double p = GlassControlMotion.TabEmberGlide.At(seconds);
        return (translateX * (1 - p), scaleX + (1 - scaleX) * p);
    }
}

/// <summary>What <see cref="TabEmberGlide.Decide"/> tells the ember to do.</summary>
public enum EmberMove
{
    /// <summary>Move to the tab's frame at once, stopping any glide.</summary>
    Place,

    /// <summary>Move to the new tab's frame, its visual springing there from the old tab's.</summary>
    Glide,

    /// <summary>Nothing changed: leave the ember, and any glide under way, alone.</summary>
    Stay,
}

/// <summary>The material under the window's panes (spec W5-D1 (b)).</summary>
public enum WindowMaterial
{
    /// <summary>The painted flat glass: opaque tiers, as the Mac under Reduce Transparency.</summary>
    Painted,
    /// <summary>Mica, with the pane's veil over it.</summary>
    Mica,
}

/// <summary>Which material the window shows.</summary>
public static class WindowMaterialChoice
{
    /// <summary>
    /// Mica only where the system reports it supported, and never under a
    /// high contrast theme, whose colours are the system's own; painted
    /// glass otherwise (W5-D1 (b), checklist D4). Acrylic was not chosen.
    /// </summary>
    public static WindowMaterial For(bool micaSupported, bool highContrast) =>
        micaSupported && !highContrast ? WindowMaterial.Mica : WindowMaterial.Painted;

    /// <summary>The window ground's brush key for a material and focus state.</summary>
    public static string GroundKey(WindowMaterial material, bool active) => (material, active) switch
    {
        (WindowMaterial.Mica, true) => "TcGlassPaneOverMaterialBrush",
        (WindowMaterial.Mica, false) => "TcGlassPaneOverMaterialInactiveBrush",
        (_, true) => "TcGlassPaneBrush",
        _ => "TcGlassPaneInactiveBrush",
    };
}
