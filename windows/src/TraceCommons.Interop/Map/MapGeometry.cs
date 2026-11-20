using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The design space fitted into the field, centred and zoomed, or scaled
/// 1.6 times about a focused tool (<c>FlowMapGeometry</c>,
/// FlowMapView.swift:764-804), with the map view's pure statics: the node
/// card's place, the label type, the folder label's cut, how wide a node is
/// drawn, and each folder rule's inks.
/// </summary>
public sealed record MapGeometry
{
    public const double FocusScale = 1.6;

    public double Scale { get; }

    public double OriginX { get; }

    public double OriginY { get; }

    public MapGeometry(MapSize size, double zoom, MapPoint? focus = null, MapSize? space = null)
    {
        MapSize design = space ?? FlowMapScene.Size;
        double fit = Math.Min(size.Width / design.Width, size.Height / design.Height);
        double cx = (size.Width - design.Width * fit) / 2, cy = (size.Height - design.Height * fit) / 2;
        if (focus is { } f)
        {
            Scale = Math.Max(0.01, fit * FocusScale);
            OriginX = cx + fit * (design.Width / 2 - FocusScale * (f.X - 50));
            OriginY = cy + fit * (design.Height / 2 - FocusScale * f.Y);
        }
        else
        {
            Scale = Math.Max(0.01, fit * zoom);
            OriginX = (size.Width - design.Width * Scale) / 2;
            OriginY = (size.Height - design.Height * Scale) / 2;
        }
    }

    public MapPoint Point(MapPoint p) => new(OriginX + p.X * Scale, OriginY + p.Y * Scale);

    /// <summary>The zoom's range and step (FlowMapView.swift:57-58).</summary>
    public const double ZoomMin = 0.6, ZoomMax = 2, ZoomStep = 0.25;

    /// <summary>The spinner drawn while the first read is out (:60).</summary>
    public const double LoadingSpinner = 20;

    /// <summary>The station's frame on the rocket map (:290).</summary>
    public const double StationWidth = 150, StationHeight = 130;

    /// <summary>The transfer card's widest (:293).</summary>
    public const double TransferCardWidth = 460;

    /// <summary>The node card's gap from its node and least margin (:719-720).</summary>
    public const double CardGap = 12, CardMargin = 8;

    /// <summary>The node card's width (<c>GlassTokens.Size.nodeCardWidth</c>).</summary>
    public const double NodeCardWidth = 240;

    /// <summary>Where a node's card is centred across the field (:727-735).</summary>
    public static double CardX(double centre, double halfWidth, double card, double field)
    {
        double right = centre + halfWidth + CardGap + card / 2;
        if (right + card / 2 + CardMargin <= field)
        {
            return right;
        }

        double left = centre - halfWidth - CardGap - card / 2;
        if (left - card / 2 >= CardMargin)
        {
            return left;
        }

        double x = field - centre > centre ? right : left;
        return Math.Min(Math.Max(card / 2 + CardMargin, x), field - card / 2 - CardMargin);
    }

    /// <summary>The card's y: beside its node, held 48 from the field's edges (:687).</summary>
    public static double CardY(double centreY, double field) => Math.Min(Math.Max(48, centreY - 24), field - 48);

    /// <summary>Half the width a node is drawn at, in design units (:707-715).</summary>
    public static double DrawnHalfWidth(MapNode node, bool traces, TracesMapStyle style)
    {
        if (!traces)
        {
            return node.Radius;
        }

        return node.Kind switch
        {
            MapNodeKind.Library => style == TracesMapStyle.Launch ? StationWidth / 2 : TransferDrawing.OrbFrame / 2,
            MapNodeKind.Hub => FlowMapScene.PadHalfWidth,
            _ => node.Radius,
        };
    }

    /// <summary>The smallest factor labels are drawn at (:470).</summary>
    public const double LabelFloor = 0.85;

    /// <summary>A node's label and sublabel sizes and offsets (:462-466).</summary>
    public static (double LabelSize, double SublabelSize, double LabelOffset, double SublabelOffset) LabelType(double scale)
    {
        double unit = Math.Max(scale, LabelFloor);
        return (11 * unit, 10 * unit, 12 * unit, 26.5 * unit);
    }

    /// <summary>A folder's name cut to 14 characters, the last an ellipsis (:580-586).</summary>
    public const int LabelLimit = 14;

    public static string ShortLabel(string label)
    {
        var info = new StringInfo(label);
        return info.LengthInTextElements > LabelLimit ? info.SubstringByTextElements(0, LabelLimit - 1) + "…" : label;
    }

    /// <summary>A node's halo gap and width (:509-512).</summary>
    public static (double Offset, double LineWidth) RingMetrics(double scale) => (4 * scale, 3 * scale);

    /// <summary>Another device's laptop against this computer's (:430).</summary>
    public const double DeviceScale = 0.45;

    /// <summary>A folder dot's fill and ring tokens by rule (:588-596).</summary>
    public static (string Fill, string Stroke) Rule(string? mode) => mode switch
    {
        MapModes.AutoUpload => ("mapRuleAutoFill", "statusOn"),
        MapModes.Ask => ("mapRuleAskFill", "statusAsk"),
        MapModes.Ignore => ("mapRuleIgnoreFill", "statusOutside"),
        _ => ("mapRuleUnsetFill", "statusOff"),
    };

    /// <summary>
    /// An arc's stroke: ink token and alpha, width factor, dash (design
    /// units, null for solid) and whether its caps are round
    /// (FlowMapView.swift:263-298). A flowing arc also draws its moving dash
    /// (<see cref="FlowingDash"/>) over this lit line.
    /// </summary>
    public static (string Ink, double Alpha, double[]? Dash, bool Round) ArcStroke(MapArcStyle style, bool healthy, bool strong)
    {
        return style switch
        {
            MapArcStyle.Quiet => ("ink", strong ? 0.5 : 0.2, null, false),
            MapArcStyle.Dashed => ("ink", strong ? 0.45 : 0.18, new[] { 5.0, 5.0 }, false),
            MapArcStyle.Shared => ("statusOn", 0.35, null, false),
            MapArcStyle.Unknown => ("white", 0.18, new[] { 0.5, 4.0 }, true),
            MapArcStyle.Flowing when !healthy => ("ink", strong ? 0.45 : 0.25, new[] { 5.0, 5.0 }, false),
            _ => ("statusOn", 0.35, null, false),
        };
    }

    /// <summary>
    /// Every stroke the Mac draws for one arc, in order, at the map's
    /// <paramref name="scale"/> (FlowMapView.swift:263-297): the line, and on
    /// a healthy flowing arc its moving dash over it. Widths and dashes are in
    /// points; <see cref="MapStroke.Moves"/> marks the dash whose phase runs.
    /// </summary>
    public static IReadOnlyList<MapStroke> ArcStrokes(MapArcStyle style, bool healthy, bool strong, double scale)
    {
        (string ink, double alpha, double[]? dash, bool round) = ArcStroke(style, healthy, strong);
        double width = ArcWidth * scale;
        var line = new MapStroke(ink, alpha, width, dash?.Select(d => d * scale).ToArray(), round, Moves: false);
        if (style != MapArcStyle.Flowing || !healthy)
        {
            return new[] { line };
        }

        var flow = new MapStroke("statusOn", 1, width * FlowingWidthFactor, FlowingDash.Select(d => d * scale).ToArray(), Round: true, Moves: true);
        return new[] { line, flow };
    }

    /// <summary>The flowing arc's moving dash: [4, 8] at 1.3 times the line, round caps (:290-295).</summary>
    public static readonly double[] FlowingDash = { 4, 8 };

    public const double FlowingWidthFactor = 1.3;

    /// <summary>Every arc's line width before scale (:269).</summary>
    public const double ArcWidth = 1.4;
}

/// <summary>One stroke of an arc: ink token and alpha (before the arc's dim), width and dash in points, round caps, and whether its dash moves.</summary>
public sealed record MapStroke(string Ink, double Alpha, double Width, double[]? Dash, bool Round, bool Moves);
