using System;

namespace TraceCommons.Interop;

/// <summary>
/// "The Turn" rasterized to a BGRA pixel buffer, for the one surface on
/// Windows that cannot take vector geometry: a notification-area icon.
/// </summary>
/// <remarks>
/// <para>
/// Everywhere else in this app the mark is
/// <c>TraceCommons.App.Controls.BrandMark</c> -- XAML paths on a 64-unit
/// Viewbox, which is why 14px and 84px are the same drawing. The shell's
/// notification area takes an <c>HICON</c>, so the mark has to become pixels
/// somewhere. It becomes them here, in the assembly that does not need
/// Windows to run, so the geometry and the scaling are exercised by tests on
/// a developer machine rather than only being looked at on a VM.
/// </para>
/// <para>
/// <b>Why not ship a .ico.</b> The tray asks for whatever size the current
/// DPI calls for -- 16, 20, 24, 32 -- and the mark is geometry, not an image.
/// A size ladder of PNGs baked into an .ico is a second description of the
/// mark that has to be kept in step with the XAML by hand. Rendering from the
/// same numbers keeps one description.
/// </para>
/// <para>
/// <b>Why single ink rather than the green/blue pair.</b> The design spec's
/// template variant -- frameless, one ink, stroke 8 rather than 7 -- exists
/// for exactly this position, and the Linux tray uses it for the same reason:
/// at 16px the frame is half a device pixel and the two brand colours are
/// four pixels of each. The caller supplies the ink, because unlike macOS the
/// Windows notification area does not recolour what it is given and the
/// taskbar can be light or dark independently of the app.
/// </para>
/// </remarks>
public static class MarkRaster
{
    /// <summary>The mark's coordinate space. Every constant below is in these units.</summary>
    private const double View = 64.0;

    /// <summary>
    /// Samples per pixel edge. The mark is axis-aligned rectangles, so 4x4
    /// coverage sampling is enough to keep a 16px stroke from crawling
    /// between DPI scales, and it costs nothing at these sizes.
    /// </summary>
    private const int SamplesPerEdge = 4;

    /// <summary>
    /// The geometry, transcribed from <c>design-import/DESIGN-SPEC.md</c>
    /// section 1.2 the same way <c>BrandMark.xaml</c> and
    /// <c>gtk/src/ui/mark.rs</c> transcribe it:
    /// <code>
    /// template  M11 28 V11 H28   stroke-width 8
    ///           M53 36 v17 H36   stroke-width 8
    /// </code>
    /// A stroke of 8 centred on those paths, mitred at the corner and butt
    /// capped at the free ends, is the four rectangles below. Written as
    /// rectangles rather than as a path walk because a rectangle is what the
    /// coverage test needs and because there is no path renderer in this
    /// assembly to walk it with.
    /// </summary>
    private static readonly Rect[] Brackets =
    {
        // The user's bracket, top left: down the left edge and along the top.
        new Rect(7, 7, 15, 28),
        new Rect(7, 7, 28, 15),

        // The agent's answer, bottom right. The same shape rotated 180
        // degrees about the centre of the 64-unit space.
        new Rect(49, 36, 57, 57),
        new Rect(36, 49, 57, 57),
    };

    /// <summary>
    /// Where a state dot goes: the top-right quadrant, which both brackets
    /// leave empty. Radius 9 of 64, which is a little under three pixels at
    /// 16px -- readable as "something is different" without competing with
    /// the mark.
    /// </summary>
    private const double DotCentreX = 52.0;
    private const double DotCentreY = 12.0;
    private const double DotRadius = 9.0;

    /// <summary>
    /// Renders the mark.
    /// </summary>
    /// <param name="sizePx">Edge length in device pixels. The icon is square.</param>
    /// <param name="inkArgb">
    /// The single ink, as 0xAARRGGBB. Callers pass the palette's
    /// <c>tc_ink</c> for the taskbar's current theme.
    /// </param>
    /// <param name="dotArgb">
    /// A state dot in the top-right corner, or null for none. Used for the
    /// attention and unhealthy states, which a Windows tray icon cannot carry
    /// as a numeric badge at these sizes -- the exact count lives in the
    /// tooltip and the menu header instead, where it is legible and where a
    /// screen reader can reach it.
    /// </param>
    /// <returns>
    /// <paramref name="sizePx"/> squared pixels of straight (not
    /// premultiplied) BGRA, top row first. That is the layout a 32bpp
    /// <c>CreateDIBSection</c> bitmap wants, and 32bpp icons are composited
    /// from the alpha channel rather than from the mask.
    /// </returns>
    public static byte[] Render(int sizePx, uint inkArgb, uint? dotArgb = null)
    {
        if (sizePx <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(sizePx));
        }

        var pixels = new byte[sizePx * sizePx * 4];
        double scale = View / sizePx;
        double step = 1.0 / SamplesPerEdge;

        for (int y = 0; y < sizePx; y++)
        {
            for (int x = 0; x < sizePx; x++)
            {
                int inkHits = 0;
                int dotHits = 0;

                for (int sy = 0; sy < SamplesPerEdge; sy++)
                {
                    for (int sx = 0; sx < SamplesPerEdge; sx++)
                    {
                        double vx = (x + (sx + 0.5) * step) * scale;
                        double vy = (y + (sy + 0.5) * step) * scale;

                        if (dotArgb is not null && InDot(vx, vy))
                        {
                            dotHits++;
                        }
                        else if (InBrackets(vx, vy))
                        {
                            inkHits++;
                        }
                    }
                }

                int total = SamplesPerEdge * SamplesPerEdge;

                // The dot is drawn over the mark rather than blended with it:
                // a sample inside the dot is never also counted as ink above,
                // so a dot that did overlap a bracket would occlude it
                // cleanly instead of muddying both.
                uint colour = dotHits > 0 && dotArgb is { } dot ? dot : inkArgb;
                int hits = dotHits > 0 ? dotHits : inkHits;
                if (hits == 0)
                {
                    continue;
                }

                byte alpha = (byte)Math.Clamp(
                    (int)Math.Round((colour >> 24 & 0xFF) * (hits / (double)total)),
                    0,
                    255);

                int offset = (y * sizePx + x) * 4;
                pixels[offset + 0] = (byte)(colour & 0xFF);          // B
                pixels[offset + 1] = (byte)(colour >> 8 & 0xFF);     // G
                pixels[offset + 2] = (byte)(colour >> 16 & 0xFF);    // R
                pixels[offset + 3] = alpha;
            }
        }

        return pixels;
    }

    private static bool InBrackets(double x, double y)
    {
        foreach (Rect rect in Brackets)
        {
            if (rect.Contains(x, y))
            {
                return true;
            }
        }

        return false;
    }

    private static bool InDot(double x, double y)
    {
        double dx = x - DotCentreX;
        double dy = y - DotCentreY;
        return dx * dx + dy * dy <= DotRadius * DotRadius;
    }

    private readonly struct Rect
    {
        private readonly double _left;
        private readonly double _top;
        private readonly double _right;
        private readonly double _bottom;

        public Rect(double left, double top, double right, double bottom)
        {
            _left = left;
            _top = top;
            _right = right;
            _bottom = bottom;
        }

        public bool Contains(double x, double y) =>
            x >= _left && x < _right && y >= _top && y < _bottom;
    }
}
