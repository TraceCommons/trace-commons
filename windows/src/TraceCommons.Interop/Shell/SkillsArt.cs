using System;
using System.Linq;
using System.Text;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// A catalog skill's cover, drawn from its id and its publisher's id
/// (<c>SkillCoverArt</c>, macos/Sources/TraceCommonsApp/Views/InsightsSkillsCover.swift).
/// Nothing is fetched: the catalog carries no artwork. The publisher picks
/// the colours and the skill picks the pattern, both from FNV-1a with the
/// MurmurHash3 finalizer, so a skill has the same cover on the Mac and here.
/// </summary>
public static class SkillCoverArt
{
    public enum Pattern { Rings = 0, Bars = 1, Dots = 2, Arcs = 3 }

    /// <summary>FNV-1a, 64 bit, over the string's UTF-8 bytes.</summary>
    public static ulong Fnv1a(string value)
    {
        ulong hash = 0xcbf2_9ce4_8422_2325;
        foreach (byte b in Encoding.UTF8.GetBytes(value))
        {
            hash ^= b;
            hash = unchecked(hash * 0x0000_0100_0000_01b3);
        }

        return hash;
    }

    /// <summary>FNV-1a then MurmurHash3's 64-bit finalizer, so near ids ("skill-1", "skill-2") do not share a cover.</summary>
    public static ulong Seed(string value)
    {
        ulong hash = Fnv1a(value);
        unchecked
        {
            hash ^= hash >> 33;
            hash *= 0xff51_afd7_ed55_8ccd;
            hash ^= hash >> 33;
            hash *= 0xc4ce_b9fe_1a85_ec53;
            hash ^= hash >> 33;
        }

        return hash;
    }

    /// <summary>
    /// The gradients a publisher can take, the same in light and dark as
    /// artwork is. Every stop holds white at 4.5:1 or more.
    /// </summary>
    public static readonly (uint From, uint To)[] Palettes =
    {
        (0x2F3FA8, 0x6B3FB8),
        (0x0E5C8F, 0x1D4FB0),
        (0x0F6B5C, 0x1B5E86),
        (0x8A3A12, 0xA8324A),
        (0x7A2E8C, 0xB0306A),
        (0x4A5A12, 0x1F6B3A),
        (0x6E5008, 0x9A3C16),
        (0x35404F, 0x4A3F6B),
    };

    /// <summary>The pattern's ink: white at 0.16 (InsightsSkillsCover.swift:120).</summary>
    public const double InkOpacity = 0.16;

    /// <summary>A tile's corner radius and a hero band's (InsightsSkillsCover.swift:170, InsightsSkillsHomeView.swift:212).</summary>
    public const double TileRadius = 9, BandRadius = 10;

    /// <summary>The tile's text shadow: black at 0.35, radius 1, 0.5 down (InsightsSkillsCover.swift:199).</summary>
    public const double ShadowOpacity = 0.35, ShadowRadius = 1, ShadowOffsetY = 0.5;

    /// <summary>The band's corner mark: black at 0.28 behind the initials (InsightsSkillsCover.swift:194).</summary>
    public const double MarkOpacity = 0.28;

    /// <summary>Everything a cover is drawn from.</summary>
    public sealed record Spec(int Palette, Pattern Pattern, double AnchorX, double AnchorY, double Step, double Angle, bool Flipped)
    {
        public static Spec For(string skillId, string publisherId)
        {
            int palette = (int)(Seed(publisherId) % (ulong)Palettes.Length);
            // The separator keeps ("ab", "c") and ("a", "bc") apart.
            ulong hash = Seed(publisherId + "\0" + skillId);
            return new Spec(
                palette,
                (Pattern)(int)(hash & 3),
                ((hash >> 8) & 0xFF) / 255.0,
                ((hash >> 16) & 0xFF) / 255.0,
                0.12 + ((hash >> 24) & 0x3F) / 63.0 * 0.12,
                ((hash >> 32) & 0xFF) / 255.0 * 70 - 35,
                ((hash >> 40) & 1) == 1);
        }
    }

    /// <summary>Up to two letters: the first letters of the first two words, or the first two of a single word.</summary>
    public static string Initials(string name)
    {
        var words = new System.Collections.Generic.List<string>();
        var current = new StringBuilder();
        foreach (Rune rune in name.EnumerateRunes())
        {
            if (Rune.IsLetterOrDigit(rune))
            {
                current.Append(rune.ToString());
            }
            else if (current.Length > 0)
            {
                words.Add(current.ToString());
                current.Clear();
            }
        }

        if (current.Length > 0) words.Add(current.ToString());
        string letters = words.Count > 1
            ? string.Concat(words.Take(2).Select(w => w.EnumerateRunes().First().ToString()))
            : words.Count == 1 ? string.Concat(words[0].EnumerateRunes().Take(2).Select(r => r.ToString())) : string.Empty;
        return letters.ToUpperInvariant();
    }

    /// <summary>One shape of a cover's pattern, in the cover's own coordinates.</summary>
    public readonly record struct Mark(bool Ellipse, double X, double Y, double Width, double Height, double Stroke);

    /// <summary>
    /// The pattern's shapes for a cover of the given size, in drawing order
    /// (<c>SkillCoverArt.draw</c>): rings and arcs are stroked ellipses, dots
    /// filled ones, and bars filled rectangles in a frame rotated by
    /// <see cref="Spec.Angle"/> about the centre.
    /// </summary>
    public static Mark[] Marks(Spec spec, double width, double height)
    {
        var marks = new System.Collections.Generic.List<Mark>();
        double unit = Math.Min(width, height);
        double reach = Math.Max(width, height) * 1.6;
        double step = Math.Max(4, unit * spec.Step);
        double ax = width * spec.AnchorX, ay = height * spec.AnchorY;
        switch (spec.Pattern)
        {
            case Pattern.Rings:
                for (double r = step; r < reach; r += step)
                    marks.Add(new Mark(true, ax - r, ay - r, r * 2, r * 2, Math.Max(1, step * 0.3)));
                break;
            case Pattern.Bars:
                for (double x = -reach + step * spec.AnchorX; x < reach; x += step)
                    marks.Add(new Mark(false, x, -reach, step * 0.45, reach * 2, 0));
                break;
            case Pattern.Dots:
                for (double y = step * (spec.AnchorY - 1); y < height + step; y += step)
                    for (double x = step * (spec.AnchorX - 1); x < width + step; x += step)
                        marks.Add(new Mark(true, x, y, step * 0.45, step * 0.45, 0));
                break;
            case Pattern.Arcs:
                double cx = spec.AnchorX < 0.5 ? 0 : width, cy = spec.AnchorY < 0.5 ? 0 : height;
                for (double r = step * (1 + spec.AnchorX); r < reach; r += step * 1.5)
                    marks.Add(new Mark(true, cx - r, cy - r, r * 2, r * 2, Math.Max(1, step * 0.45)));
                break;
        }

        return marks.ToArray();
    }

    /// <summary>WCAG contrast of white on a colour.</summary>
    public static double ContrastWithWhite(uint rgb)
    {
        static double Linear(double c) => c <= 0.03928 ? c / 12.92 : Math.Pow((c + 0.055) / 1.055, 2.4);
        double r = ((rgb >> 16) & 0xFF) / 255.0, g = ((rgb >> 8) & 0xFF) / 255.0, b = (rgb & 0xFF) / 255.0;
        double luminance = 0.2126 * Linear(r) + 0.7152 * Linear(g) + 0.0722 * Linear(b);
        return 1.05 / (luminance + 0.05);
    }
}

/// <summary>
/// What moves on Skills Home (<c>SkillsHomeMotion</c>,
/// InsightsSkillsHomeView.swift:56-76, spec W20 Animation coverage row).
/// Everything is off when the system turns animations off, and while the
/// window's clocks are stopped: the feed is then still and complete on its
/// first frame.
/// </summary>
public sealed record SkillsHomeMotion(bool Reduce)
{
    public static readonly SkillsHomeMotion Still = new(true);

    /// <summary>A card rises 1.5 under the pointer, easing out over 0.15 s (InsightsSkillsHomeView.swift:74, :439).</summary>
    public const double HoverLift = 1.5, HoverSeconds = 0.15;

    /// <summary>A card outside the viewport's centre fades to 0.6 and scales to 0.985 (:443, <c>scrollTransition</c>).</summary>
    public const double ScrollOpacity = 0.6, ScrollScale = 0.985;

    /// <summary>Bars rise from 2% of their height, easing out over 0.5 s, once (:75, :472, :491).</summary>
    public const double GrowFrom = 0.02, GrowSeconds = 0.5;

    /// <summary>A story opening in place, and a count rolling: <c>.snappy(duration: 0.25)</c> (:73); see <see cref="Snappy"/>.</summary>
    public const double ExpandSeconds = 0.25;

    /// <summary>The Home column's widest, and the hero band's height (:91-92).</summary>
    public const double ColumnMaximum = 560, HeroBand = 96;

    /// <summary>A tile's size beside a story, and a cover strip's minimum and height (:94, :246, :252).</summary>
    public const double MarkSize = 40, StripMinimum = 84, StripHeight = 48;

    public bool GrowsBars => !Reduce;
    public bool Lifts => !Reduce;
    public bool Scrolls => !Reduce;
    public bool RollsNumbers => !Reduce;

    /// <summary>
    /// <c>.snappy(duration: 0.25)</c> is <c>Spring(response: 0.25,
    /// dampingRatio: 0.85)</c>; a throwaway SwiftUI probe on macOS printed
    /// both numbers (2026-11-19).
    /// </summary>
    public const double SnappyResponse = 0.25, SnappyDamping = 0.85;

    /// <summary>That spring's <c>settlingDuration</c>, printed by the same probe: how long an expand or a roll runs.</summary>
    public const double SnappySettling = 0.4688122063285237;

    /// <summary>
    /// The snappy spring's value <paramref name="seconds"/> after it starts,
    /// with SwiftUI's slight overshoot; exactly 1 once it has settled. Used
    /// for the expand and the number roll.
    /// </summary>
    public static double Snappy(double seconds) =>
        seconds >= SnappySettling ? 1 : MonitorMotion.Spring(SnappyResponse, SnappyDamping, seconds);

    /// <summary><c>.easeOut</c>: CoreAnimation's ease-out curve, (0, 0, 0.58, 1).</summary>
    public static double EaseOut(double t) => MonitorMotion.CubicBezier(0, 0, 0.58, 1, t);
}
