using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>A key spline's two control points, as WinUI's <c>KeySpline</c> takes them.</summary>
public readonly record struct SplinePoints(double X1, double Y1, double X2, double Y2);

/// <summary>One property of a named template part set to a value.</summary>
public readonly record struct PartValue(string Part, string Property, double Value);

/// <summary>
/// A visual state's resting values. A state with none keeps what its
/// template sets (the Disabled opacity, a token resource); motion is all
/// that is built here.
/// </summary>
public sealed record StatePlan(string State, IReadOnlyList<PartValue> Values);

/// <summary>
/// A visual transition: into <see cref="To"/> (from any state when
/// <see cref="From"/> is null), each value reached over
/// <see cref="Seconds"/> on <see cref="Spline"/>.
/// </summary>
public sealed record TransitionPlan(string? From, string? To, double Seconds, SplinePoints Spline, IReadOnlyList<PartValue> Values);

/// <summary>A visual state group's states and transitions, built into a re-templated control.</summary>
public sealed record GroupPlan(string Group, IReadOnlyList<StatePlan> States, IReadOnlyList<TransitionPlan> Transitions);

/// <summary>A template part's size and leading inset.</summary>
public readonly record struct PartFrame(string Part, double Width, double Height, double Left, double Right);

/// <summary>The kinds of re-templated control whose motion is built from these plans.</summary>
public enum GlassTemplateKind
{
    /// <summary>The switch (GlassToggleStyle, standard and Settings kinds): 40 by 24, an 18 knob inset 3, ease-out 0.22 s.</summary>
    Switch,
    /// <summary>The watch switch (GlassToggleStyle .watch): 38 by 22, an 18 knob inset 2, ease-out 0.15 s.</summary>
    WatchSwitch,
    /// <summary>The first run's small switch (FirstRunStage.swift:463-492): 30 by 18, a 14 knob inset 2, 0.2 s token curve.</summary>
    FirstRunSwitch,
    /// <summary>The refresh pill (Containers.swift GlassRefreshButton): hover fill and press wash, fast token curve.</summary>
    RefreshPill,
}

/// <summary>
/// The re-templated controls' motion as data (spec W5-D3 (a)): every state
/// value, transition duration, curve and part size the templates draw.
/// The templates in GlassControls.xaml carry only their states' names; the
/// app's <c>GlassTemplate</c> builds these plans into them, so each number
/// drawn is the one these tests pin, never a literal in the XAML.
/// </summary>
public static class GlassTemplateMotion
{
    /// <summary>A curve's control points; a spring has none and is never a template transition.</summary>
    public static SplinePoints Spline(MotionSpec spec) => spec.Curve switch
    {
        MotionCurve.Token => new(GlassTokens.Motion.EaseX1, GlassTokens.Motion.EaseY1, GlassTokens.Motion.EaseX2, GlassTokens.Motion.EaseY2),
        MotionCurve.EaseOut => new(GlassMotion.EaseOutX1, GlassMotion.EaseOutY1, GlassMotion.EaseOutX2, GlassMotion.EaseOutY2),
        _ => throw new ArgumentException("a spring is not a key spline", nameof(spec)),
    };

    /// <summary>A switch kind's size, knob and inset, and its knob's slide.</summary>
    public static (double Width, double Height, double Knob, double Inset, MotionSpec Slide) SwitchShape(GlassTemplateKind kind) => kind switch
    {
        GlassTemplateKind.WatchSwitch => Shape(GlassSwitchKind.Watch),
        // FirstRunStage.swift:472-482: 30 by 18, knob 14, padding 2,
        // GlassMotion.timed(duration: 0.2), the token curve.
        GlassTemplateKind.FirstRunSwitch => (30, 18, 14, 2, MotionSpec.Token(0.2)),
        _ => Shape(GlassSwitchKind.Settings),
    };

    private static (double, double, double, double, MotionSpec) Shape(GlassSwitchKind kind)
    {
        (double width, double height, double knob, double inset) = GlassControlMotion.SwitchGeometry(kind);
        return (width, height, knob, inset, GlassControlMotion.SwitchKnob(kind));
    }

    /// <summary>How far the knob slides from off to on: the track less both insets and the knob.</summary>
    public static double KnobTravel(GlassTemplateKind kind)
    {
        (double width, _, double knob, double inset, _) = SwitchShape(kind);
        return width - 2 * inset - knob;
    }

    /// <summary>The parts of a switch template sized from its kind.</summary>
    public static IReadOnlyList<PartFrame> Frames(GlassTemplateKind kind)
    {
        if (kind == GlassTemplateKind.RefreshPill)
        {
            return Array.Empty<PartFrame>();
        }

        (double width, double height, double knob, double inset, _) = SwitchShape(kind);
        return new[]
        {
            new PartFrame("SwitchArea", width, height, 0, 0),
            // The drag bounds: the track less its insets, so the knob's
            // travel is the bounds less the knob (the stock part's contract).
            new PartFrame("SwitchKnobBounds", double.NaN, double.NaN, inset, inset),
            new PartFrame("SwitchKnob", knob, knob, inset, 0),
        };
    }

    /// <summary>
    /// The space between a switch and its label (Controls.swift GlassToggleStyle:
    /// <c>HStack(spacing: Space.s6)</c>; FirstRunSwitch: <c>HStack(spacing: 10)</c>).
    /// </summary>
    public static double LabelGap(GlassTemplateKind kind) =>
        kind == GlassTemplateKind.FirstRunSwitch ? 10 : GlassTokens.Space.S6;

    /// <summary>
    /// Whether the label stands before the switch. The Mac's GlassToggleStyle
    /// puts its label first and the switch on the trailing side; the first
    /// run's small switch puts the switch first.
    /// </summary>
    public static bool LabelLeads(GlassTemplateKind kind) => kind != GlassTemplateKind.FirstRunSwitch;

    /// <summary>Every visual state group a kind's template runs.</summary>
    public static IReadOnlyList<GroupPlan> Groups(GlassTemplateKind kind) =>
        kind == GlassTemplateKind.RefreshPill ? RefreshGroups() : SwitchGroups(kind);

    private static IReadOnlyList<GroupPlan> SwitchGroups(GlassTemplateKind kind)
    {
        MotionSpec slide = SwitchShape(kind).Slide;
        SplinePoints slideCurve = Spline(slide);
        double travel = KnobTravel(kind);
        PartValue[] off = { new("KnobTranslateTransform", "X", 0), new("OnTrack", "Opacity", 0), new("KnobOn", "Opacity", 0) };
        PartValue[] on = { new("KnobTranslateTransform", "X", travel), new("OnTrack", "Opacity", 1), new("KnobOn", "Opacity", 1) };

        // The press: a 0.08 black wash over the track on the fast token
        // curve (GlassPress.swift:108-127). The first run's switch is a
        // plain button with no press style, so it has none.
        var commonTransitions = new List<TransitionPlan>();
        if (kind != GlassTemplateKind.FirstRunSwitch)
        {
            MotionSpec press = GlassControlMotion.Press;
            commonTransitions.Add(new(null, "Pressed", press.Duration, Spline(press), new[] { new PartValue("PressWash", "Opacity", 1) }));
            commonTransitions.Add(new("Pressed", null, press.Duration, Spline(press), new[] { new PartValue("PressWash", "Opacity", 0) }));
        }

        return new[]
        {
            new GroupPlan(
                "CommonStates",
                new[]
                {
                    new StatePlan("Normal", Array.Empty<PartValue>()),
                    new StatePlan("PointerOver", Array.Empty<PartValue>()),
                    new StatePlan("Pressed", kind == GlassTemplateKind.FirstRunSwitch ? Array.Empty<PartValue>() : new[] { new PartValue("PressWash", "Opacity", 1) }),
                },
                commonTransitions),
            new GroupPlan(
                "ToggleStates",
                new[] { new StatePlan("Dragging", Array.Empty<PartValue>()), new StatePlan("Off", off), new StatePlan("On", on) },
                new[]
                {
                    new TransitionPlan(null, "On", slide.Duration, slideCurve, on),
                    new TransitionPlan(null, "Off", slide.Duration, slideCurve, off),
                }),
        };
    }

    private static IReadOnlyList<GroupPlan> RefreshGroups()
    {
        // GlassHover.swift:40 (hover fill 0 to 1, fast) and GlassPress.swift
        // (the 0.08 wash, fast), both on the token curve.
        MotionSpec hover = GlassControlMotion.HoverFade, press = GlassControlMotion.Press;
        PartValue[] normal = { new("HoverFill", "Opacity", 0), new("PressWash", "Opacity", 0) };
        PartValue[] over = { new("HoverFill", "Opacity", 1), new("PressWash", "Opacity", 0) };
        PartValue[] pressed = { new("HoverFill", "Opacity", 1), new("PressWash", "Opacity", 1) };
        return new[]
        {
            new GroupPlan(
                "CommonStates",
                new[]
                {
                    new StatePlan("Normal", normal),
                    new StatePlan("PointerOver", over),
                    new StatePlan("Pressed", pressed),
                },
                new[]
                {
                    new TransitionPlan(null, "PointerOver", hover.Duration, Spline(hover), over),
                    new TransitionPlan(null, "Pressed", press.Duration, Spline(press), pressed),
                    new TransitionPlan(null, "Normal", hover.Duration, Spline(hover), normal),
                }),
        };
    }
}
