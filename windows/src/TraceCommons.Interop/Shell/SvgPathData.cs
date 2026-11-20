using System;
using System.Collections.Generic;
using System.Globalization;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// SVG path data (the <c>d</c> attribute) parsed into absolute steps: a port
/// of the Mac's <c>SVGPathData.parse</c> (macos/Sources/TCDesign/Logos/SVGPathData.swift:14-117)
/// and its arc conversion (:121-178) and scanner (:180-246). It covers the
/// whole grammar the tool logos use: M L H V C S Q T A Z, absolute and
/// relative, implicit repeats, numbers run together (<c>.08-.23</c>) and arc
/// flags written as bare digits (<c>0 00-.975 0</c>). Arcs become cubics of
/// at most a quarter turn each, as on the Mac. A quadratic, which CoreGraphics
/// keeps as its own element, is raised to the equal cubic here (no logo has
/// one at 409066936).
/// </summary>
/// <remarks>
/// Returns null on malformed data rather than a partial path, as the Mac
/// does, so a logo that would draw half a mark draws nothing.
/// </remarks>
public static class SvgPathData
{
    public static IReadOnlyList<PathStep>? Parse(string data)
    {
        var steps = new List<PathStep>();
        var s = new Scanner(data);
        double cx = 0, cy = 0, startX = 0, startY = 0;
        (double X, double Y)? lastCubic = null, lastQuad = null;
        char? command = null;
        while (true)
        {
            s.SkipSeparators();
            if (s.Peek() is not char c)
            {
                break;
            }

            if (Scanner.IsCommand(c))
            {
                command = c;
                s.Advance();
            }
            else if (command is null)
            {
                return null;
            }

            char cmd = command!.Value;
            bool relative = char.IsLower(cmd);
            double ox = relative ? cx : 0, oy = relative ? cy : 0;
            switch (char.ToLowerInvariant(cmd))
            {
                case 'm':
                {
                    if (s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    cx = x + ox;
                    cy = y + oy;
                    startX = cx;
                    startY = cy;
                    steps.Add(new PathStep(PathStepKind.Move, 0, 0, 0, 0, cx, cy));
                    command = relative ? 'l' : 'L';
                    lastCubic = lastQuad = null;
                    break;
                }

                case 'l':
                {
                    if (s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    cx = x + ox;
                    cy = y + oy;
                    steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, cx, cy));
                    lastCubic = lastQuad = null;
                    break;
                }

                case 'h':
                {
                    if (s.Number() is not double x)
                    {
                        return null;
                    }

                    cx = relative ? cx + x : x;
                    steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, cx, cy));
                    lastCubic = lastQuad = null;
                    break;
                }

                case 'v':
                {
                    if (s.Number() is not double y)
                    {
                        return null;
                    }

                    cy = relative ? cy + y : y;
                    steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, cx, cy));
                    lastCubic = lastQuad = null;
                    break;
                }

                case 'c':
                {
                    if (s.Point() is not (double x1, double y1) || s.Point() is not (double x2, double y2) || s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    steps.Add(new PathStep(PathStepKind.Cubic, x1 + ox, y1 + oy, x2 + ox, y2 + oy, x + ox, y + oy));
                    lastCubic = (x2 + ox, y2 + oy);
                    cx = x + ox;
                    cy = y + oy;
                    lastQuad = null;
                    break;
                }

                case 's':
                {
                    if (s.Point() is not (double x2, double y2) || s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    (double X, double Y) c1 = lastCubic is { } lc ? (2 * cx - lc.X, 2 * cy - lc.Y) : (cx, cy);
                    steps.Add(new PathStep(PathStepKind.Cubic, c1.X, c1.Y, x2 + ox, y2 + oy, x + ox, y + oy));
                    lastCubic = (x2 + ox, y2 + oy);
                    cx = x + ox;
                    cy = y + oy;
                    lastQuad = null;
                    break;
                }

                case 'q':
                {
                    if (s.Point() is not (double qx, double qy) || s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    AddQuad(steps, cx, cy, qx + ox, qy + oy, x + ox, y + oy);
                    lastQuad = (qx + ox, qy + oy);
                    cx = x + ox;
                    cy = y + oy;
                    lastCubic = null;
                    break;
                }

                case 't':
                {
                    if (s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    (double X, double Y) q = lastQuad is { } lq ? (2 * cx - lq.X, 2 * cy - lq.Y) : (cx, cy);
                    AddQuad(steps, cx, cy, q.X, q.Y, x + ox, y + oy);
                    lastQuad = q;
                    cx = x + ox;
                    cy = y + oy;
                    lastCubic = null;
                    break;
                }

                case 'a':
                {
                    if (s.Number() is not double rx || s.Number() is not double ry || s.Number() is not double rotation
                        || s.Flag() is not bool largeArc || s.Flag() is not bool sweep || s.Point() is not (double x, double y))
                    {
                        return null;
                    }

                    double ex = x + ox, ey = y + oy;
                    AddArc(steps, cx, cy, ex, ey, rx, ry, rotation, largeArc, sweep);
                    cx = ex;
                    cy = ey;
                    lastCubic = lastQuad = null;
                    break;
                }

                case 'z':
                    steps.Add(new PathStep(PathStepKind.Close, 0, 0, 0, 0, startX, startY));
                    cx = startX;
                    cy = startY;
                    lastCubic = lastQuad = null;
                    // Z takes no arguments; what follows must be a new command.
                    command = null;
                    break;
                default:
                    return null;
            }
        }

        return steps.Count == 0 ? null : steps;
    }

    private static void AddQuad(List<PathStep> steps, double x0, double y0, double qx, double qy, double x, double y) =>
        steps.Add(new PathStep(
            PathStepKind.Cubic,
            x0 + 2.0 / 3 * (qx - x0), y0 + 2.0 / 3 * (qy - y0),
            x + 2.0 / 3 * (qx - x), y + 2.0 / 3 * (qy - y),
            x, y));

    /// <summary>SVG's endpoint arc in centre form (SVG 1.1 appendix F.6), as SVGPathData.swift:121-178.</summary>
    private static void AddArc(
        List<PathStep> steps, double p0x, double p0y, double p1x, double p1y,
        double rx, double ry, double rotation, bool largeArc, bool sweep)
    {
        rx = Math.Abs(rx);
        ry = Math.Abs(ry);
        if (p0x == p1x && p0y == p1y)
        {
            return;
        }

        if (rx == 0 || ry == 0)
        {
            steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, p1x, p1y));
            return;
        }

        double phi = rotation * Math.PI / 180;
        double cosPhi = Math.Cos(phi), sinPhi = Math.Sin(phi);
        double dx = (p0x - p1x) / 2, dy = (p0y - p1y) / 2;
        double x1 = cosPhi * dx + sinPhi * dy;
        double y1 = -sinPhi * dx + cosPhi * dy;
        double lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
        if (lambda > 1)
        {
            rx *= Math.Sqrt(lambda);
            ry *= Math.Sqrt(lambda);
        }

        double numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
        double denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        double coefficient = Math.Sqrt(Math.Max(0, numerator) / denominator);
        if (largeArc == sweep)
        {
            coefficient = -coefficient;
        }

        double cx1 = coefficient * rx * y1 / ry;
        double cy1 = -coefficient * ry * x1 / rx;
        double ccx = cosPhi * cx1 - sinPhi * cy1 + (p0x + p1x) / 2;
        double ccy = sinPhi * cx1 + cosPhi * cy1 + (p0y + p1y) / 2;

        static double Angle(double ux, double uy, double vx, double vy)
        {
            double sign = ux * vy - uy * vx < 0 ? -1 : 1;
            double dot = ux * vx + uy * vy;
            double length = Math.Sqrt(ux * ux + uy * uy) * Math.Sqrt(vx * vx + vy * vy);
            return sign * Math.Acos(Math.Min(1, Math.Max(-1, dot / length)));
        }

        double theta1 = Angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry);
        double delta = Angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
        if (!sweep && delta > 0)
        {
            delta -= 2 * Math.PI;
        }

        if (sweep && delta < 0)
        {
            delta += 2 * Math.PI;
        }

        int segments = Math.Max(1, (int)Math.Ceiling(Math.Abs(delta) / (Math.PI / 2)));
        double step = delta / segments;
        double k = 4.0 / 3 * Math.Tan(step / 4);
        (double X, double Y) Point(double t) => (
            ccx + rx * Math.Cos(t) * cosPhi - ry * Math.Sin(t) * sinPhi,
            ccy + rx * Math.Cos(t) * sinPhi + ry * Math.Sin(t) * cosPhi);
        (double X, double Y) Derivative(double t) => (
            -rx * Math.Sin(t) * cosPhi - ry * Math.Cos(t) * sinPhi,
            -rx * Math.Sin(t) * sinPhi + ry * Math.Cos(t) * cosPhi);
        double at = theta1;
        for (int index = 0; index < segments; index++)
        {
            double next = at + step;
            var start = Point(at);
            var d0 = Derivative(at);
            var pn = Point(next);
            var d1 = Derivative(next);
            var end = index == segments - 1 ? (p1x, p1y) : pn;
            steps.Add(new PathStep(
                PathStepKind.Cubic,
                start.X + d0.X * k, start.Y + d0.Y * k,
                pn.X - d1.X * k, pn.Y - d1.Y * k,
                end.Item1, end.Item2));
            at = next;
        }
    }

    private sealed class Scanner(string data)
    {
        private int _index;

        public static bool IsCommand(char c) => "mlhvcsqtazMLHVCSQTAZ".IndexOf(c) >= 0;

        public char? Peek() => _index < data.Length ? data[_index] : null;

        public void Advance() => _index++;

        public void SkipSeparators()
        {
            while (Peek() is ' ' or ',' or '\n' or '\t' or '\r')
            {
                _index++;
            }
        }

        /// <summary>An arc flag: a single 0 or 1, which may run straight into the next number.</summary>
        public bool? Flag()
        {
            SkipSeparators();
            if (Peek() is not ('0' or '1'))
            {
                return null;
            }

            return data[_index++] == '1';
        }

        public (double X, double Y)? Point() =>
            Number() is double x && Number() is double y ? (x, y) : null;

        /// <summary>One number; a second '.' or a sign ends it.</summary>
        public double? Number()
        {
            SkipSeparators();
            int start = _index;
            if (Peek() is '-' or '+')
            {
                _index++;
            }

            int digits = 0;
            while (Peek() is char d && d >= '0' && d <= '9')
            {
                _index++;
                digits++;
            }

            if (Peek() == '.')
            {
                _index++;
                while (Peek() is char d && d >= '0' && d <= '9')
                {
                    _index++;
                    digits++;
                }
            }

            if (digits == 0)
            {
                _index = start;
                return null;
            }

            if (Peek() is 'e' or 'E')
            {
                int mark = _index;
                _index++;
                if (Peek() is '-' or '+')
                {
                    _index++;
                }

                int exponent = 0;
                while (Peek() is char d && d >= '0' && d <= '9')
                {
                    _index++;
                    exponent++;
                }

                if (exponent == 0)
                {
                    _index = mark;
                }
            }

            return double.TryParse(data.AsSpan(start, _index - start), NumberStyles.Float, CultureInfo.InvariantCulture, out double value)
                ? value : null;
        }
    }
}
