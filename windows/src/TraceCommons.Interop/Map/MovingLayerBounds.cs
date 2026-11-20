using System;
using System.Collections.Generic;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.Map;

/// <summary>
/// Where the map's moving layer draws while it rests: the arcs with their
/// flowing dashes, the library's turning frame and the rocket at its rest
/// (no transfer playing, the pile still in the still layer). The layer's
/// canvas is sized to this box instead of the whole pane, so each of its
/// frames (30 a second while the dashes flow) clears, draws and composes
/// only these pixels. On a machine with no GPU (WARP) that work is done on
/// the CPU, in proportion to the pixels. The box's edges are on whole device
/// pixels and the layer is drawn through the box's offset, so every part
/// lands on the same pixels as on a whole-pane canvas: nothing looks
/// different. Windows only: the Mac draws through Core Animation.
/// </summary>
public static class MovingLayerBounds
{
    /// <summary>Room round every part, in DIPs: anti-aliasing reaches past an edge.</summary>
    public const double Slack = 2;

    /// <summary>A box this share of the pane or more is not worth a smaller canvas.</summary>
    public const double WholeShare = 0.85;

    /// <summary>
    /// The box holding every part in <paramref name="parts"/> with
    /// <see cref="Slack"/> round it, snapped outwards to whole device pixels
    /// at <paramref name="rasterScale"/> and kept inside the pane; or null,
    /// meaning the whole pane: when <paramref name="whole"/> (a run or a
    /// moving pile draws anywhere), when there is nothing to draw, or when
    /// the box is most of the pane anyway.
    /// </summary>
    public static LayoutRect? Resting(MapSize pane, double rasterScale, bool whole, IEnumerable<LayoutRect> parts)
    {
        if (whole || pane.Width <= 0 || pane.Height <= 0)
        {
            return null;
        }

        double left = double.PositiveInfinity, top = double.PositiveInfinity;
        double right = double.NegativeInfinity, bottom = double.NegativeInfinity;
        foreach (LayoutRect p in parts)
        {
            left = Math.Min(left, p.X - Slack);
            top = Math.Min(top, p.Y - Slack);
            right = Math.Max(right, p.X + p.Width + Slack);
            bottom = Math.Max(bottom, p.Y + p.Height + Slack);
        }

        if (double.IsInfinity(left))
        {
            return null;
        }

        double s = rasterScale > 0 ? rasterScale : 1;
        left = Math.Max(0, Math.Floor(left * s) / s);
        top = Math.Max(0, Math.Floor(top * s) / s);
        right = Math.Min(Math.Ceiling(pane.Width * s) / s, Math.Ceiling(right * s) / s);
        bottom = Math.Min(Math.Ceiling(pane.Height * s) / s, Math.Ceiling(bottom * s) / s);
        if (right <= left || bottom <= top)
        {
            return null;
        }

        if ((right - left) * (bottom - top) >= WholeShare * pane.Width * pane.Height)
        {
            return null;
        }

        return new LayoutRect(left, top, right - left, bottom - top);
    }

    /// <summary>
    /// A cubic Bezier's box, from its four points (the curve stays inside
    /// their hull), grown by <paramref name="halfWidth"/> for its stroke.
    /// </summary>
    public static LayoutRect Curve(MapPoint a, MapPoint c1, MapPoint c2, MapPoint b, double halfWidth)
    {
        double left = Math.Min(Math.Min(a.X, c1.X), Math.Min(c2.X, b.X)) - halfWidth;
        double top = Math.Min(Math.Min(a.Y, c1.Y), Math.Min(c2.Y, b.Y)) - halfWidth;
        double right = Math.Max(Math.Max(a.X, c1.X), Math.Max(c2.X, b.X)) + halfWidth;
        double bottom = Math.Max(Math.Max(a.Y, c1.Y), Math.Max(c2.Y, b.Y)) + halfWidth;
        return new LayoutRect(left, top, right - left, bottom - top);
    }

    /// <summary>A <paramref name="width"/> by <paramref name="height"/> box centred on <paramref name="centre"/>.</summary>
    public static LayoutRect Around(MapPoint centre, double width, double height) =>
        new(centre.X - width / 2, centre.Y - height / 2, width, height);
}
