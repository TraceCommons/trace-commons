using System;
using System.Collections.Generic;
using System.Globalization;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.Map;

/// <summary>
/// SVG path data into absolute drawing steps, a port of the Mac's
/// <c>SVGPathData.parse</c> (macos/Sources/TCDesign/Logos/SVGPathData.swift):
/// M L H V C S Q T A Z, absolute and relative, implicit repeats, numbers run
/// together and bare-digit arc flags. Arcs become cubics of at most a quarter
/// turn, as there; a quadratic becomes the cubic that draws the same curve.
/// Null on malformed data, so a logo draws nothing rather than half a mark.
/// </summary>
public static class SvgPathData
{
    public static IReadOnlyList<PathStep>? Parse(string data)
    {
        var steps = new List<PathStep>();
        var scan = new Scanner(data);
        double cx = 0, cy = 0, sx = 0, sy = 0;
        (double X, double Y)? lastCubic = null, lastQuad = null;
        char? command = null;

        void Line(double x, double y) => steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, x, y));
        void Cubic(double x1, double y1, double x2, double y2, double x, double y) =>
            steps.Add(new PathStep(PathStepKind.Cubic, x1, y1, x2, y2, x, y));
        void Quad(double qx, double qy, double x, double y, double fromX, double fromY) =>
            Cubic(fromX + 2.0 / 3 * (qx - fromX), fromY + 2.0 / 3 * (qy - fromY), x + 2.0 / 3 * (qx - x), y + 2.0 / 3 * (qy - y), x, y);

        while (true)
        {
            scan.SkipSeparators();
            if (scan.Peek() is not { } c)
            {
                break;
            }

            if (Scanner.IsCommand(c))
            {
                command = c;
                scan.Advance();
            }
            else if (command is null)
            {
                return null;
            }

            char cmd = command!.Value;
            bool relative = cmd >= 'a';
            double ox = relative ? cx : 0, oy = relative ? cy : 0;
            switch (char.ToLowerInvariant(cmd))
            {
                case 'm':
                {
                    if (scan.Point() is not { } p)
                    {
                        return null;
                    }

                    cx = p.X + ox;
                    cy = p.Y + oy;
                    sx = cx;
                    sy = cy;
                    steps.Add(new PathStep(PathStepKind.Move, 0, 0, 0, 0, cx, cy));
                    command = relative ? 'l' : 'L';
                    lastCubic = null;
                    lastQuad = null;
                    break;
                }

                case 'l':
                {
                    if (scan.Point() is not { } p)
                    {
                        return null;
                    }

                    cx = p.X + ox;
                    cy = p.Y + oy;
                    Line(cx, cy);
                    lastCubic = null;
                    lastQuad = null;
                    break;
                }

                case 'h':
                {
                    if (scan.Number() is not { } x)
                    {
                        return null;
                    }

                    cx = relative ? cx + x : x;
                    Line(cx, cy);
                    lastCubic = null;
                    lastQuad = null;
                    break;
                }

                case 'v':
                {
                    if (scan.Number() is not { } y)
                    {
                        return null;
                    }

                    cy = relative ? cy + y : y;
                    Line(cx, cy);
                    lastCubic = null;
                    lastQuad = null;
                    break;
                }

                case 'c':
                {
                    if (scan.Point() is not { } c1 || scan.Point() is not { } c2 || scan.Point() is not { } p)
                    {
                        return null;
                    }

                    Cubic(c1.X + ox, c1.Y + oy, c2.X + ox, c2.Y + oy, p.X + ox, p.Y + oy);
                    lastCubic = (c2.X + ox, c2.Y + oy);
                    cx = p.X + ox;
                    cy = p.Y + oy;
                    lastQuad = null;
                    break;
                }

                case 's':
                {
                    if (scan.Point() is not { } c2 || scan.Point() is not { } p)
                    {
                        return null;
                    }

                    (double X, double Y) c1 = lastCubic is { } lc ? (cx * 2 - lc.X, cy * 2 - lc.Y) : (cx, cy);
                    Cubic(c1.X, c1.Y, c2.X + ox, c2.Y + oy, p.X + ox, p.Y + oy);
                    lastCubic = (c2.X + ox, c2.Y + oy);
                    cx = p.X + ox;
                    cy = p.Y + oy;
                    lastQuad = null;
                    break;
                }

                case 'q':
                {
                    if (scan.Point() is not { } q || scan.Point() is not { } p)
                    {
                        return null;
                    }

                    Quad(q.X + ox, q.Y + oy, p.X + ox, p.Y + oy, cx, cy);
                    lastQuad = (q.X + ox, q.Y + oy);
                    cx = p.X + ox;
                    cy = p.Y + oy;
                    lastCubic = null;
                    break;
                }

                case 't':
                {
                    if (scan.Point() is not { } p)
                    {
                        return null;
                    }

                    (double X, double Y) q = lastQuad is { } lq ? (cx * 2 - lq.X, cy * 2 - lq.Y) : (cx, cy);
                    Quad(q.X, q.Y, p.X + ox, p.Y + oy, cx, cy);
                    lastQuad = q;
                    cx = p.X + ox;
                    cy = p.Y + oy;
                    lastCubic = null;
                    break;
                }

                case 'a':
                {
                    if (scan.Number() is not { } rx || scan.Number() is not { } ry || scan.Number() is not { } rotation
                        || scan.Flag() is not { } large || scan.Flag() is not { } sweep || scan.Point() is not { } p)
                    {
                        return null;
                    }

                    double ex = p.X + ox, ey = p.Y + oy;
                    AddArc(steps, cx, cy, ex, ey, rx, ry, rotation, large, sweep);
                    cx = ex;
                    cy = ey;
                    lastCubic = null;
                    lastQuad = null;
                    break;
                }

                case 'z':
                    steps.Add(new PathStep(PathStepKind.Close, 0, 0, 0, 0, sx, sy));
                    cx = sx;
                    cy = sy;
                    lastCubic = null;
                    lastQuad = null;
                    command = null;
                    break;
                default:
                    return null;
            }
        }

        return steps.Count == 0 ? null : steps;
    }

    /// <summary>SVGPathData.swift:110-176, the endpoint arc as cubics.</summary>
    private static void AddArc(List<PathStep> steps, double x0, double y0, double x1, double y1, double rx, double ry, double rotation, bool largeArc, bool sweep)
    {
        rx = Math.Abs(rx);
        ry = Math.Abs(ry);
        if (x0 == x1 && y0 == y1)
        {
            return;
        }

        if (rx == 0 || ry == 0)
        {
            steps.Add(new PathStep(PathStepKind.Line, 0, 0, 0, 0, x1, y1));
            return;
        }

        double phi = rotation * Math.PI / 180;
        double cosPhi = Math.Cos(phi), sinPhi = Math.Sin(phi);
        double dx = (x0 - x1) / 2, dy = (y0 - y1) / 2;
        double px = cosPhi * dx + sinPhi * dy;
        double py = -sinPhi * dx + cosPhi * dy;
        double lambda = px * px / (rx * rx) + py * py / (ry * ry);
        if (lambda > 1)
        {
            rx *= Math.Sqrt(lambda);
            ry *= Math.Sqrt(lambda);
        }

        double numerator = rx * rx * ry * ry - rx * rx * py * py - ry * ry * px * px;
        double denominator = rx * rx * py * py + ry * ry * px * px;
        double coefficient = Math.Sqrt(Math.Max(0, numerator) / denominator);
        if (largeArc == sweep)
        {
            coefficient = -coefficient;
        }

        double cx1 = coefficient * rx * py / ry;
        double cy1 = -coefficient * ry * px / rx;
        double cx = cosPhi * cx1 - sinPhi * cy1 + (x0 + x1) / 2;
        double cy = sinPhi * cx1 + cosPhi * cy1 + (y0 + y1) / 2;

        static double Angle(double ux, double uy, double vx, double vy)
        {
            double sign = ux * vy - uy * vx < 0 ? -1 : 1;
            double dot = ux * vx + uy * vy;
            double length = Math.Sqrt(ux * ux + uy * uy) * Math.Sqrt(vx * vx + vy * vy);
            return sign * Math.Acos(Math.Min(1, Math.Max(-1, dot / length)));
        }

        double theta1 = Angle(1, 0, (px - cx1) / rx, (py - cy1) / ry);
        double delta = Angle((px - cx1) / rx, (py - cy1) / ry, (-px - cx1) / rx, (-py - cy1) / ry);
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
        (double X, double Y) Point(double t) =>
            (cx + rx * Math.Cos(t) * cosPhi - ry * Math.Sin(t) * sinPhi, cy + rx * Math.Cos(t) * sinPhi + ry * Math.Sin(t) * cosPhi);
        (double X, double Y) Derivative(double t) =>
            (-rx * Math.Sin(t) * cosPhi - ry * Math.Cos(t) * sinPhi, -rx * Math.Sin(t) * sinPhi + ry * Math.Cos(t) * cosPhi);
        double t0 = theta1;
        for (int i = 0; i < segments; i++)
        {
            double next = t0 + step;
            (double X, double Y) start = Point(t0);
            (double X, double Y) end = i == segments - 1 ? (x1, y1) : Point(next);
            (double X, double Y) d0 = Derivative(t0), d1 = Derivative(next), pn = Point(next);
            steps.Add(new PathStep(PathStepKind.Cubic,
                start.X + d0.X * k, start.Y + d0.Y * k, pn.X - d1.X * k, pn.Y - d1.Y * k, end.X, end.Y));
            t0 = next;
        }
    }

    private sealed class Scanner
    {
        private readonly string _s;
        private int _i;

        public Scanner(string s) => _s = s;

        public static bool IsCommand(char c) => char.ToLowerInvariant(c) is 'm' or 'l' or 'h' or 'v' or 'c' or 's' or 'q' or 't' or 'a' or 'z';

        public char? Peek() => _i < _s.Length ? _s[_i] : null;

        public void Advance() => _i++;

        public void SkipSeparators()
        {
            while (_i < _s.Length && _s[_i] is ' ' or ',' or '\n' or '\t' or '\r')
            {
                _i++;
            }
        }

        public bool? Flag()
        {
            SkipSeparators();
            if (Peek() is not ('0' or '1'))
            {
                return null;
            }

            return _s[_i++] == '1';
        }

        public (double X, double Y)? Point() => Number() is { } x && Number() is { } y ? (x, y) : null;

        public double? Number()
        {
            SkipSeparators();
            int start = _i;
            if (Peek() is '-' or '+')
            {
                _i++;
            }

            int digits = 0;
            while (Peek() is >= '0' and <= '9')
            {
                _i++;
                digits++;
            }

            if (Peek() == '.')
            {
                _i++;
                while (Peek() is >= '0' and <= '9')
                {
                    _i++;
                    digits++;
                }
            }

            if (digits == 0)
            {
                _i = start;
                return null;
            }

            if (Peek() is 'e' or 'E')
            {
                int mark = _i;
                _i++;
                if (Peek() is '-' or '+')
                {
                    _i++;
                }

                int exponent = 0;
                while (Peek() is >= '0' and <= '9')
                {
                    _i++;
                    exponent++;
                }

                if (exponent == 0)
                {
                    _i = mark;
                }
            }

            return double.TryParse(_s.AsSpan(start, _i - start), NumberStyles.Float, CultureInfo.InvariantCulture, out double v) ? v : null;
        }
    }
}
