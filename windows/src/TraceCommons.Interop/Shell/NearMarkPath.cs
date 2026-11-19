using System;
using System.Collections.Generic;
using System.Globalization;

namespace TraceCommons.Interop.Shell;

/// <summary>One drawing step of a path, in absolute coordinates.</summary>
public readonly record struct PathStep(PathStepKind Kind, double X1, double Y1, double X2, double Y2, double X, double Y);

public enum PathStepKind
{
    Move,
    Line,
    Cubic,
    Close,
}

/// <summary>
/// The NEAR AI "N", the app's mark (owner decision G-3 (a), 2026-11-18: "The
/// NEAR AI N everywhere: title bar, tray, first-run stage and the MSIX
/// tiles"). The path data is the Mac's, verbatim from
/// macos/Sources/TCDesign/Logos/GlassBrandMark.swift:14-15, which records its
/// own source: the brand kit's <c>Tertiary/02_White &amp; Color/tile-white-color.svg</c>
/// (the blue path) in the tile's 400 by 396 coordinates.
/// </summary>
/// <remarks>
/// Parsed here once so the title bar's XAML geometry and the Win2D renders draw
/// from one parse, and so the parse is tested against the Mac's
/// (<c>SVGPathData.parse</c>) without Windows.
/// </remarks>
public static class NearMarkPath
{
    /// <summary>GlassBrandMark.swift:12: the mark's bounds in the tile's coordinates.</summary>
    public static readonly LayoutRect ViewBox = new(82.96, 82.13, 234.08, 231.72);

    /// <summary>GlassBrandMark.swift:14-15, verbatim.</summary>
    public const string Data =
        "M292.106,82.1357C283.445,82.1357,275.379,86.5848,270.847,93.9047L221.912,165.811C220.318,168.188,220.971,171.382,223.353,172.96C225.292,174.234,227.866,174.082,229.633,172.58L277.799,131.227C278.605,130.504,279.834,130.581,280.545,131.379C280.872,131.74,281.044,132.216,281.044,132.691V262.169C281.044,263.233,280.18,264.089,279.085,264.089C278.509,264.089,277.952,263.842,277.587,263.404L132.014,90.8817C127.271,85.3489,120.318,82.1548,112.982,82.1357H107.893C94.1229,82.1357,82.9648,93.1822,82.9648,106.814V289.167C82.9648,302.799,94.1229,313.846,107.893,313.846C116.554,313.846,124.62,309.397,129.153,302.077L178.087,230.17C179.681,227.793,179.028,224.599,176.646,223.021C174.707,221.747,172.133,221.899,170.366,223.401L122.2,264.754C121.394,265.477,120.165,265.401,119.454,264.602C119.128,264.241,118.955,263.766,118.955,263.29V133.775C118.955,132.71,119.819,131.854,120.914,131.854C121.49,131.854,122.047,132.102,122.412,132.539L267.966,305.1C272.709,310.632,279.662,313.827,286.998,313.846H292.087C305.857,313.846,317.034,302.799,317.034,289.167V106.833C317.034,93.2012,305.876,82.1548,292.106,82.1548V82.1357Z";

    private static readonly Lazy<IReadOnlyList<PathStep>> UnitSteps = new(() => ToUnit(Parse(Data)));

    /// <summary>
    /// The mark moved so its top-left is the origin and scaled to a unit square,
    /// aspect kept (<c>GlassBrandMark.unit</c>, GlassBrandMark.swift:20-27).
    /// </summary>
    public static IReadOnlyList<PathStep> Unit => UnitSteps.Value;

    /// <summary>
    /// Parses absolute <c>M L H V C Z</c> path data with comma or space
    /// separators: the commands the mark uses. Anything else throws, so a
    /// changed source fails its test rather than drawing part of a mark.
    /// </summary>
    public static IReadOnlyList<PathStep> Parse(string data)
    {
        var steps = new List<PathStep>();
        int i = 0;
        double cx = 0, cy = 0;
        char command = '\0';
        while (true)
        {
            SkipSeparators(data, ref i);
            if (i >= data.Length)
            {
                break;
            }

            if (char.IsLetter(data[i]))
            {
                command = data[i++];
            }

            switch (command)
            {
                case 'M':
                    cx = Number(data, ref i);
                    cy = Number(data, ref i);
                    steps.Add(new PathStep(PathStepKind.Move, 0, 0, 0, 0, cx, cy));
                    command = 'L';
                    break;
                case 'L':
                    cx = Number(data, ref i);
                    cy = Number(data, ref i);
                    steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, cx, cy));
                    break;
                case 'H':
                    cx = Number(data, ref i);
                    steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, cx, cy));
                    break;
                case 'V':
                    cy = Number(data, ref i);
                    steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, cx, cy));
                    break;
                case 'C':
                    double x1 = Number(data, ref i), y1 = Number(data, ref i);
                    double x2 = Number(data, ref i), y2 = Number(data, ref i);
                    cx = Number(data, ref i);
                    cy = Number(data, ref i);
                    steps.Add(new PathStep(PathStepKind.Cubic, x1, y1, x2, y2, cx, cy));
                    break;
                case 'Z':
                    steps.Add(new PathStep(PathStepKind.Close, 0, 0, 0, 0, cx, cy));
                    command = '\0';
                    break;
                default:
                    throw new FormatException($"unsupported path command '{command}' at {i}");
            }
        }

        return steps;
    }

    private static IReadOnlyList<PathStep> ToUnit(IReadOnlyList<PathStep> steps)
    {
        double scale = 1 / Math.Max(ViewBox.Width, ViewBox.Height);
        double Tx(double x) => (x - ViewBox.X) * scale;
        double Ty(double y) => (y - ViewBox.Y) * scale;
        var unit = new PathStep[steps.Count];
        for (int k = 0; k < steps.Count; k++)
        {
            PathStep s = steps[k];
            unit[k] = s with { X1 = Tx(s.X1), Y1 = Ty(s.Y1), X2 = Tx(s.X2), Y2 = Ty(s.Y2), X = Tx(s.X), Y = Ty(s.Y) };
        }

        return unit;
    }

    /// <summary>
    /// The exact bounds of the steps' curves (not their control points), as
    /// CoreGraphics' <c>boundingRect</c> reports them.
    /// </summary>
    public static LayoutRect Bounds(IReadOnlyList<PathStep> steps)
    {
        double minX = double.MaxValue, minY = double.MaxValue, maxX = double.MinValue, maxY = double.MinValue;
        void Add(double x, double y)
        {
            minX = Math.Min(minX, x);
            minY = Math.Min(minY, y);
            maxX = Math.Max(maxX, x);
            maxY = Math.Max(maxY, y);
        }

        double px = 0, py = 0;
        foreach (PathStep s in steps)
        {
            if (s.Kind == PathStepKind.Cubic)
            {
                for (int n = 0; n <= 400; n++)
                {
                    double t = n / 400.0, u = 1 - t;
                    Add(
                        u * u * u * px + 3 * u * u * t * s.X1 + 3 * u * t * t * s.X2 + t * t * t * s.X,
                        u * u * u * py + 3 * u * u * t * s.Y1 + 3 * u * t * t * s.Y2 + t * t * t * s.Y);
                }
            }
            else if (s.Kind != PathStepKind.Close)
            {
                Add(s.X, s.Y);
            }

            px = s.X;
            py = s.Y;
        }

        return new LayoutRect(minX, minY, maxX - minX, maxY - minY);
    }

    private static void SkipSeparators(string data, ref int i)
    {
        while (i < data.Length && (data[i] == ',' || char.IsWhiteSpace(data[i])))
        {
            i++;
        }
    }

    private static double Number(string data, ref int i)
    {
        SkipSeparators(data, ref i);
        int start = i;
        if (i < data.Length && (data[i] == '-' || data[i] == '+'))
        {
            i++;
        }

        while (i < data.Length && (char.IsDigit(data[i]) || data[i] == '.'))
        {
            i++;
        }

        return double.Parse(data.AsSpan(start, i - start), NumberStyles.Float, CultureInfo.InvariantCulture);
    }
}
