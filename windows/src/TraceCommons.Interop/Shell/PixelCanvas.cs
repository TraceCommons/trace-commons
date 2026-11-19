using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>A straight (not premultiplied) colour, each channel 0 to 1.</summary>
public readonly record struct Rgba(double R, double G, double B, double A)
{
    /// <summary>From 0xAARRGGBB, the form the generated inks take.</summary>
    public static Rgba FromArgb(uint argb) => new(
        ((argb >> 16) & 0xFF) / 255.0,
        ((argb >> 8) & 0xFF) / 255.0,
        (argb & 0xFF) / 255.0,
        (argb >> 24) / 255.0);

    /// <summary>SwiftUI's <c>.opacity(_)</c>: the alpha scaled.</summary>
    public Rgba WithOpacity(double opacity) => this with { A = A * Math.Clamp(opacity, 0, 1) };

    /// <summary>Straight-line mix of two colours, alpha included.</summary>
    public static Rgba Mix(Rgba a, Rgba b, double t) => new(
        a.R + (b.R - a.R) * t,
        a.G + (b.G - a.G) * t,
        a.B + (b.B - a.B) * t,
        a.A + (b.A - a.A) * t);
}

/// <summary>
/// A shape the canvas can fill: for a sample row at <c>y</c>, the x ranges it
/// covers. Every shape on Home's coins and the tray mark is one span per row
/// (a disc, a capsule, a rectangle, a turned ellipse) except a ring and the
/// N's polygon, which give several.
/// </summary>
public abstract class CanvasShape
{
    /// <summary>The rows the shape can touch, in canvas pixels.</summary>
    public abstract (double Top, double Bottom) Rows { get; }

    /// <summary>Adds the x ranges covered on row <paramref name="y"/> to <paramref name="spans"/>.</summary>
    public abstract void Spans(double y, List<(double From, double To)> spans);

    /// <summary>A disc of radius <paramref name="r"/> about (cx, cy).</summary>
    public static CanvasShape Circle(double cx, double cy, double r) => new EllipseShape(cx, cy, r, r, 0);

    /// <summary>An ellipse with half-axes rx, ry about (cx, cy), turned by <paramref name="angle"/> radians (clockwise, y down).</summary>
    public static CanvasShape Ellipse(double cx, double cy, double rx, double ry, double angle) => new EllipseShape(cx, cy, rx, ry, angle);

    /// <summary>An axis-aligned rectangle.</summary>
    public static CanvasShape Rect(double x, double y, double w, double h) => new RectShape(x, y, w, h);

    /// <summary>A capsule: a rectangle whose shorter sides are half circles (SwiftUI's <c>Capsule</c>).</summary>
    public static CanvasShape Capsule(double x, double y, double w, double h) => new CapsuleShape(x, y, w, h);

    /// <summary>The band between two capsules: a capsule's stroke of <paramref name="line"/> centred on its edge.</summary>
    public static CanvasShape CapsuleStroke(double x, double y, double w, double h, double line) =>
        new DifferenceShape(
            new CapsuleShape(x - line / 2, y - line / 2, w + line, h + line),
            new CapsuleShape(x + line / 2, y + line / 2, w - line, h - line));

    /// <summary>A circle's stroke of <paramref name="line"/> centred on radius <paramref name="r"/>.</summary>
    public static CanvasShape Ring(double cx, double cy, double r, double line) =>
        new DifferenceShape(new EllipseShape(cx, cy, r + line / 2, r + line / 2, 0), new EllipseShape(cx, cy, r - line / 2, r - line / 2, 0));

    /// <summary>Closed polygons filled by the nonzero winding rule (SwiftUI's default fill).</summary>
    public static CanvasShape Polygons(IReadOnlyList<IReadOnlyList<(double X, double Y)>> rings) => new PolygonShape(rings);

    private sealed class EllipseShape(double cx, double cy, double rx, double ry, double angle) : CanvasShape
    {
        private readonly double _cos = Math.Cos(angle);
        private readonly double _sin = Math.Sin(angle);
        private readonly double _reach = Math.Max(rx, ry);

        public override (double Top, double Bottom) Rows => (cy - _reach, cy + _reach);

        public override void Spans(double y, List<(double From, double To)> spans)
        {
            if (rx <= 0 || ry <= 0)
            {
                return;
            }

            // Points (x, y) inside: with u, v the point in the ellipse's own
            // axes, (u/rx)^2 + (v/ry)^2 <= 1, a quadratic in x.
            double dy = y - cy;
            double a = (_cos * _cos) / (rx * rx) + (_sin * _sin) / (ry * ry);
            double b = 2 * dy * _cos * _sin * (1 / (rx * rx) - 1 / (ry * ry));
            double c = dy * dy * ((_sin * _sin) / (rx * rx) + (_cos * _cos) / (ry * ry)) - 1;
            double disc = b * b - 4 * a * c;
            if (disc < 0)
            {
                return;
            }

            double root = Math.Sqrt(disc);
            spans.Add((cx + (-b - root) / (2 * a), cx + (-b + root) / (2 * a)));
        }
    }

    private sealed class RectShape(double x, double y, double w, double h) : CanvasShape
    {
        public override (double Top, double Bottom) Rows => (y, y + h);

        public override void Spans(double row, List<(double From, double To)> spans)
        {
            if (row >= y && row < y + h && w > 0)
            {
                spans.Add((x, x + w));
            }
        }
    }

    private sealed class CapsuleShape(double x, double y, double w, double h) : CanvasShape
    {
        public override (double Top, double Bottom) Rows => (y, y + h);

        public override void Spans(double row, List<(double From, double To)> spans)
        {
            if (w <= 0 || h <= 0 || row < y || row >= y + h)
            {
                return;
            }

            double r = Math.Min(w, h) / 2;
            if (w >= h)
            {
                double dy = row - (y + r);
                double half = Math.Sqrt(Math.Max(0, r * r - dy * dy));
                spans.Add((x + r - half, x + w - r + half));
            }
            else
            {
                // Taller than wide: half circles top and bottom.
                double centre = Math.Clamp(row, y + r, y + h - r);
                double dy = row - centre;
                double half = Math.Sqrt(Math.Max(0, r * r - dy * dy));
                spans.Add((x + r - half, x + r + half));
            }
        }
    }

    private sealed class DifferenceShape(CanvasShape outer, CanvasShape inner) : CanvasShape
    {
        private readonly List<(double, double)> _outer = new();
        private readonly List<(double, double)> _inner = new();

        public override (double Top, double Bottom) Rows => outer.Rows;

        public override void Spans(double y, List<(double From, double To)> spans)
        {
            _outer.Clear();
            _inner.Clear();
            outer.Spans(y, _outer);
            inner.Spans(y, _inner);
            foreach ((double from, double to) in _outer)
            {
                double at = from;
                foreach ((double cutFrom, double cutTo) in _inner)
                {
                    if (cutTo <= at || cutFrom >= to)
                    {
                        continue;
                    }

                    if (cutFrom > at)
                    {
                        spans.Add((at, cutFrom));
                    }

                    at = Math.Max(at, cutTo);
                }

                if (at < to)
                {
                    spans.Add((at, to));
                }
            }
        }
    }

    private sealed class PolygonShape : CanvasShape
    {
        private readonly (double X0, double Y0, double X1, double Y1)[] _edges;
        private readonly (double Top, double Bottom) _rows;
        private readonly List<(double X, int Winding)> _crossings = new();

        public PolygonShape(IReadOnlyList<IReadOnlyList<(double X, double Y)>> rings)
        {
            var edges = new List<(double, double, double, double)>();
            double top = double.MaxValue, bottom = double.MinValue;
            foreach (IReadOnlyList<(double X, double Y)> ring in rings)
            {
                for (int i = 0; i < ring.Count; i++)
                {
                    (double x0, double y0) = ring[i];
                    (double x1, double y1) = ring[(i + 1) % ring.Count];
                    top = Math.Min(top, y0);
                    bottom = Math.Max(bottom, y0);
                    if (y0 != y1)
                    {
                        edges.Add((x0, y0, x1, y1));
                    }
                }
            }

            _edges = edges.ToArray();
            _rows = (top, bottom);
        }

        public override (double Top, double Bottom) Rows => _rows;

        public override void Spans(double y, List<(double From, double To)> spans)
        {
            _crossings.Clear();
            foreach ((double x0, double y0, double x1, double y1) in _edges)
            {
                bool down = y1 > y0;
                double lo = down ? y0 : y1, hi = down ? y1 : y0;
                if (y < lo || y >= hi)
                {
                    continue;
                }

                double x = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
                _crossings.Add((x, down ? 1 : -1));
            }

            _crossings.Sort((a, b) => a.X.CompareTo(b.X));
            int winding = 0;
            for (int i = 0; i < _crossings.Count; i++)
            {
                int before = winding;
                winding += _crossings[i].Winding;
                if (before == 0 && winding != 0)
                {
                    double from = _crossings[i].X;
                    int j = i + 1;
                    int w = winding;
                    for (; j < _crossings.Count; j++)
                    {
                        w += _crossings[j].Winding;
                        if (w == 0)
                        {
                            break;
                        }
                    }

                    if (j < _crossings.Count)
                    {
                        spans.Add((from, _crossings[j].X));
                    }

                    i = j;
                    winding = 0;
                }
            }
        }
    }
}

/// <summary>
/// A small software canvas: 4 by 4 samples per pixel, source-over and
/// destination-out compositing, premultiplied inside and straight BGRA out.
/// What the tray icon, the taskbar burst and Home's pixel glyph are drawn
/// with, so every one of them is drawn and tested off Windows, from the same
/// numbers the Mac draws with.
/// </summary>
public sealed class PixelCanvas
{
    /// <summary>Samples per pixel edge.</summary>
    public const int Samples = 4;

    private readonly double[] _px;
    private readonly List<(double From, double To)> _spans = new();
    private readonly int[] _hits;
    private readonly double[] _acc;

    public PixelCanvas(int width, int height)
    {
        if (width <= 0 || height <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(width));
        }

        Width = width;
        Height = height;
        _px = new double[width * height * 4];
        _hits = new int[width * height];
        _acc = new double[width * height * 4];
    }

    public int Width { get; }

    public int Height { get; }

    /// <summary>Fills <paramref name="shape"/> with one colour.</summary>
    public void Fill(CanvasShape shape, Rgba colour) => Composite(shape, colour, null, clear: false);

    /// <summary>Fills <paramref name="shape"/> with a colour per sample point (a gradient).</summary>
    public void Fill(CanvasShape shape, Func<double, double, Rgba> shader) => Composite(shape, default, shader, clear: false);

    /// <summary>Destination-out: clears what is under <paramref name="shape"/> by its coverage.</summary>
    public void Clear(CanvasShape shape) => Composite(shape, new Rgba(0, 0, 0, 1), null, clear: true);

    /// <summary>Source-over of <paramref name="source"/> with its top-left at (x, y), rounded to whole pixels.</summary>
    public void Paste(PixelCanvas source, double x, double y)
    {
        int ox = (int)Math.Round(x), oy = (int)Math.Round(y);
        for (int sy = 0; sy < source.Height; sy++)
        {
            int ty = sy + oy;
            if (ty < 0 || ty >= Height)
            {
                continue;
            }

            for (int sx = 0; sx < source.Width; sx++)
            {
                int tx = sx + ox;
                if (tx < 0 || tx >= Width)
                {
                    continue;
                }

                int s = (sy * source.Width + sx) * 4;
                BlendPremultiplied(ty * Width + tx, source._px[s], source._px[s + 1], source._px[s + 2], source._px[s + 3]);
            }
        }
    }

    /// <summary>A canvas from straight BGRA, top row first (an icon's pixels).</summary>
    public static PixelCanvas FromBgra(byte[] bgra, int width, int height)
    {
        var canvas = new PixelCanvas(width, height);
        for (int p = 0; p < width * height; p++)
        {
            int i = p * 4;
            double a = bgra[i + 3] / 255.0;
            canvas._px[i] = bgra[i + 2] / 255.0 * a;
            canvas._px[i + 1] = bgra[i + 1] / 255.0 * a;
            canvas._px[i + 2] = bgra[i] / 255.0 * a;
            canvas._px[i + 3] = a;
        }

        return canvas;
    }

    /// <summary>The pixel's straight colour.</summary>
    public Rgba Pixel(int x, int y)
    {
        int i = (y * Width + x) * 4;
        double a = _px[i + 3];
        return a <= 0 ? new Rgba(0, 0, 0, 0) : new Rgba(_px[i] / a, _px[i + 1] / a, _px[i + 2] / a, a);
    }

    /// <summary>Straight BGRA, top row first: what a 32bpp DIB and a WriteableBitmap take.</summary>
    /// <param name="premultiplied">WriteableBitmap wants premultiplied; a DIB for an icon wants straight.</param>
    public byte[] ToBgra(bool premultiplied = false)
    {
        var bytes = new byte[Width * Height * 4];
        for (int p = 0; p < Width * Height; p++)
        {
            int i = p * 4;
            double a = Math.Clamp(_px[i + 3], 0, 1);
            double k = premultiplied ? 1 : (a > 0 ? 1 / a : 0);
            bytes[i] = Byte(_px[i + 2] * k);
            bytes[i + 1] = Byte(_px[i + 1] * k);
            bytes[i + 2] = Byte(_px[i] * k);
            bytes[i + 3] = Byte(a);
        }

        return bytes;
    }

    private static byte Byte(double v) => (byte)Math.Clamp((int)Math.Round(v * 255), 0, 255);

    private void Composite(CanvasShape shape, Rgba colour, Func<double, double, Rgba>? shader, bool clear)
    {
        (double top, double bottom) = shape.Rows;
        int firstRow = Math.Max(0, (int)Math.Floor(top * Samples));
        int lastRow = Math.Min(Height * Samples - 1, (int)Math.Ceiling(bottom * Samples));
        if (firstRow > lastRow)
        {
            return;
        }

        const double total = Samples * Samples;
        int minPx = int.MaxValue, maxPx = int.MinValue, minPy = int.MaxValue, maxPy = int.MinValue;
        for (int sy = firstRow; sy <= lastRow; sy++)
        {
            double y = (sy + 0.5) / Samples;
            _spans.Clear();
            shape.Spans(y, _spans);
            int py = sy / Samples;
            foreach ((double from, double to) in _spans)
            {
                // Sample columns whose centres fall inside [from, to).
                int sx0 = Math.Max(0, (int)Math.Ceiling(from * Samples - 0.5));
                int sx1 = Math.Min(Width * Samples - 1, (int)Math.Ceiling(to * Samples - 0.5) - 1);
                for (int sx = sx0; sx <= sx1; sx++)
                {
                    int px = sx / Samples;
                    int p = py * Width + px;
                    _hits[p]++;
                    if (shader is not null)
                    {
                        // Summed premultiplied, so a gradient's pixel is the
                        // average of its samples, composited once.
                        Rgba c = shader((sx + 0.5) / Samples, y);
                        double a = Math.Clamp(c.A, 0, 1);
                        _acc[p * 4] += c.R * a;
                        _acc[p * 4 + 1] += c.G * a;
                        _acc[p * 4 + 2] += c.B * a;
                        _acc[p * 4 + 3] += a;
                    }

                    minPx = Math.Min(minPx, px);
                    maxPx = Math.Max(maxPx, px);
                    minPy = Math.Min(minPy, py);
                    maxPy = Math.Max(maxPy, py);
                }
            }
        }

        if (minPx == int.MaxValue)
        {
            return;
        }

        for (int py = minPy; py <= maxPy; py++)
        {
            for (int px = minPx; px <= maxPx; px++)
            {
                int p = py * Width + px;
                int hits = _hits[p];
                if (hits == 0)
                {
                    continue;
                }

                _hits[p] = 0;
                double coverage = Math.Min(1, hits / total);
                if (shader is not null)
                {
                    int j = p * 4;
                    BlendPremultiplied(p, _acc[j] / total, _acc[j + 1] / total, _acc[j + 2] / total, Math.Min(1, _acc[j + 3] / total));
                    _acc[j] = _acc[j + 1] = _acc[j + 2] = _acc[j + 3] = 0;
                }
                else if (clear)
                {
                    int i = p * 4;
                    double keep = 1 - coverage;
                    _px[i] *= keep;
                    _px[i + 1] *= keep;
                    _px[i + 2] *= keep;
                    _px[i + 3] *= keep;
                }
                else
                {
                    Blend(p, colour, coverage);
                }
            }
        }
    }

    private void BlendPremultiplied(int p, double r, double g, double b, double a)
    {
        int i = p * 4;
        double keep = 1 - a;
        _px[i] = r + _px[i] * keep;
        _px[i + 1] = g + _px[i + 1] * keep;
        _px[i + 2] = b + _px[i + 2] * keep;
        _px[i + 3] = a + _px[i + 3] * keep;
    }

    /// <summary>Source-over of <paramref name="c"/> at <paramref name="coverage"/> onto pixel <paramref name="p"/>.</summary>
    private void Blend(int p, Rgba c, double coverage)
    {
        double a = Math.Clamp(c.A, 0, 1) * coverage;
        if (a <= 0)
        {
            return;
        }

        int i = p * 4;
        double keep = 1 - a;
        _px[i] = c.R * a + _px[i] * keep;
        _px[i + 1] = c.G * a + _px[i + 1] * keep;
        _px[i + 2] = c.B * a + _px[i + 2] * keep;
        _px[i + 3] = a + _px[i + 3] * keep;
    }
}
