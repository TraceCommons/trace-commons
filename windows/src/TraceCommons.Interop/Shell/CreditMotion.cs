using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Generated;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// One coin of a burst (<c>GlassCoinBurst.Coin</c>, CoinBurst.swift:62-77):
/// the way it flies (radians, clockwise from the right, y down), how fast it
/// leaves (points per second), its face's width, how it turns end over end,
/// the lean of that axis, and how late it leaves the centre.
/// </summary>
public readonly record struct Coin(double Angle, double Speed, double Diameter, double Spin, double Phase, double Tilt, double Delay);

/// <summary>A point offset, in points (y down).</summary>
public readonly record struct Offset(double Dx, double Dy);

/// <summary>
/// The gold coins that burst out of Home's Balance tile when NEAR AI credit
/// arrives (<c>GlassCoinBurst</c>, macos/Sources/TCDesign/Components/CoinBurst.swift).
/// Every number is the Mac's; the tests pin them to a probe compiled from
/// that Swift.
/// </summary>
public static class CoinBurst
{
    /// <summary>CoinBurst.swift:30: start to the last coin gone.</summary>
    public const double Duration = 1.7;

    /// <summary>CoinBurst.swift:32.</summary>
    public const int Count = 44;

    /// <summary>CoinBurst.swift:34: how far past the card's edges the coins may fly.</summary>
    public const double Spill = 120;

    /// <summary>CoinBurst.swift:36: the moment drawn under Reduce Motion.</summary>
    public const double StillAt = 0.45;

    /// <summary>CoinBurst.swift:40-41.</summary>
    public const double Drag = 3.4;
    public const double Gravity = 50;

    /// <summary>CoinBurst.swift:44.</summary>
    public const double Squash = 0.72;

    /// <summary>CoinBurst.swift:46.</summary>
    public const double FadeFrom = 0.55;

    /// <summary>CoinBurst.swift:48-49.</summary>
    public const double FlashDuration = 0.28;
    public const double FlashRadius = 46;

    /// <summary>How long a coin trails sparks, and their spacing (CoinBurst.swift:170-180).</summary>
    public const double SparkWhile = 0.55;
    public const double SparkStep = 0.035;

    /// <summary>The burst's coins (CoinBurst.swift:81): the same every time.</summary>
    public static IReadOnlyList<Coin> Coins { get; } = Enumerable.Range(0, Count).Select(CoinAt).ToArray();

    /// <summary>
    /// A repeatable number in 0..1 for <paramref name="index"/> and
    /// <paramref name="salt"/> (CoinBurst.swift:84-87).
    /// </summary>
    public static double Noise(int index, double salt)
    {
        double x = Math.Sin(index * 12.9898 + salt * 78.233) * 43758.5453;
        return x - Math.Floor(x);
    }

    /// <summary>CoinBurst.swift:89-99.</summary>
    public static Coin CoinAt(int index)
    {
        double slot = (double)index / Count;
        return new Coin(
            Angle: (slot + (Noise(index, 1) - 0.5) / Count) * 2 * Math.PI,
            Speed: 220 + Noise(index, 2) * 200,
            Diameter: 3.5 + Noise(index, 3) * 2.5,
            Spin: 1.4 + Noise(index, 4) * 2.2,
            Phase: Noise(index, 5),
            Tilt: (Noise(index, 6) - 0.5) * 1.2,
            Delay: Noise(index, 7) * 0.08);
    }

    /// <summary>
    /// Where a coin is <paramref name="t"/> seconds in, from the centre:
    /// out along its way, slowed by drag, pulled down; <paramref name="sideways"/>
    /// shortens its reach to the right only (CoinBurst.swift:105-111).
    /// </summary>
    public static Offset OffsetOf(Coin coin, double t, double sideways = 1)
    {
        double cosine = Math.Cos(coin.Angle);
        double flown = Math.Max(0, t - coin.Delay);
        double reach = coin.Speed * (1 - Math.Exp(-Drag * flown)) / Drag;
        double fall = 0.5 * Gravity * flown * flown;
        return new Offset(cosine * reach * (cosine > 0 ? sideways : 1), Math.Sin(coin.Angle) * reach * Squash + fall);
    }

    /// <summary>CoinBurst.swift:115: the farthest any coin flies side to side.</summary>
    public static double Farthest { get; } = Coins.Max(c => c.Speed) / Drag;

    /// <summary>CoinBurst.swift:117: the widest coin.</summary>
    public static double Widest { get; } = Coins.Max(c => c.Diameter);

    /// <summary>CoinBurst.swift:125-127: how much of its reach a coin flies right when the cut-off edge is <paramref name="room"/> from the centre.</summary>
    public static double Sideways(double room) => Math.Max(0, Math.Min(1, (room - Widest) / Farthest));

    /// <summary>CoinBurst.swift:131-135: whole while they fly, fading from <see cref="FadeFrom"/>, gone at <see cref="Duration"/>.</summary>
    public static double Opacity(double t)
    {
        double from = Duration * FadeFrom;
        if (!(t > from))
        {
            return 1;
        }

        return Math.Max(0, 1 - (t - from) / (Duration - from));
    }
}

/// <summary>
/// The burst on the taskbar button, in the Mac's Dock numbers
/// (<c>GlassDockCoinBurst</c>, macos/Sources/TCDesign/Components/DockCoinBurst.swift):
/// a 128-point tile, 24 coins, its own drag and gravity, a larger flash and a
/// pulse of the icon itself.
/// </summary>
public static class DockCoinBurst
{
    /// <summary>DockCoinBurst.swift:30.</summary>
    public const double TileSize = 128;

    /// <summary>DockCoinBurst.swift:31-32.</summary>
    public const double Duration = CoinBurst.Duration;
    public const int Count = 24;

    /// <summary>DockCoinBurst.swift:34-39.</summary>
    public const double Drag = 3.8;
    public const double Gravity = 32;
    public const double FlashDuration = 0.32;
    public const double FlashRadius = 56;
    public const double PulseDuration = 0.36;
    public const double PulsePeak = 0.045;

    /// <summary>DockCoinBurst.swift:42.</summary>
    public static IReadOnlyList<Coin> Coins { get; } = Enumerable.Range(0, Count).Select(CoinAt).ToArray();

    /// <summary>DockCoinBurst.swift:44-55.</summary>
    public static Coin CoinAt(int index)
    {
        double slot = (double)index / Count;
        return new Coin(
            Angle: (slot + (CoinBurst.Noise(index, 11) - 0.5) / Count) * 2 * Math.PI,
            Speed: 120 + CoinBurst.Noise(index, 12) * 90,
            Diameter: 6.0 + CoinBurst.Noise(index, 13) * 4.5,
            Spin: 1.5 + CoinBurst.Noise(index, 14) * 2.0,
            Phase: CoinBurst.Noise(index, 15),
            Tilt: (CoinBurst.Noise(index, 16) - 0.5) * 1.2,
            Delay: CoinBurst.Noise(index, 17) * 0.08);
    }

    /// <summary>DockCoinBurst.swift:59-62: the icon's pulse; 1 when still or settled.</summary>
    public static double IconScale(double t, bool flash)
    {
        if (!flash || !(t > 0) || !(t < PulseDuration))
        {
            return 1;
        }

        return 1 + PulsePeak * Math.Sin(Math.PI * t / PulseDuration);
    }

    /// <summary>DockCoinBurst.swift:66-77: from the tile's centre, scaled to a tile of <paramref name="tile"/>.</summary>
    public static Offset OffsetOf(Coin coin, double t, double tile = TileSize)
    {
        double flown = Math.Max(0, t - coin.Delay);
        double reach = coin.Speed * (1 - Math.Exp(-Drag * flown)) / Drag;
        double fall = 0.5 * Gravity * flown * flown;
        double scale = tile / TileSize;
        return new Offset(Math.Cos(coin.Angle) * reach * scale, (Math.Sin(coin.Angle) * reach + fall) * scale);
    }

    /// <summary>
    /// One frame of the taskbar burst over <paramref name="baseIcon"/> (a
    /// square canvas of the icon's size), as <c>GlassDockCoinBurst.body</c>
    /// and <c>drawOverlay</c> (DockCoinBurst.swift:80-142) draw it: the icon
    /// pulsed about its centre, the flash, then the coins and sparks.
    /// </summary>
    public static PixelCanvas Render(Func<int, PixelCanvas> baseIcon, int size, double t, bool flash)
    {
        double scale = IconScale(t, flash);
        int iconPx = Math.Max(1, (int)Math.Round(size * scale));
        PixelCanvas icon = baseIcon(iconPx);
        var canvas = new PixelCanvas(size, size);
        double inset = (size - iconPx) / 2.0;
        canvas.Paste(icon, inset, inset);
        if (!(t >= 0) || !(t < Duration))
        {
            return canvas;
        }

        double unit = size / TileSize;
        double cx = size / 2.0, cy = size / 2.0;
        if (flash && t < FlashDuration)
        {
            double left = 1 - t / FlashDuration;
            double radius = FlashRadius * unit * (0.35 + 0.65 * (t / FlashDuration));
            CoinDrawing.Flash(canvas, cx, cy, radius, 0.8 * left, 0.35 * left);
        }

        double opacity = CoinBurst.Opacity(t);
        foreach (Coin coin in Coins)
        {
            double flown = t - coin.Delay;
            if (!(flown > 0))
            {
                continue;
            }

            Offset at = OffsetOf(coin, t, size);
            if (flown < CoinBurst.SparkWhile)
            {
                for (int step = 1; step <= 3; step++)
                {
                    Offset back = OffsetOf(coin, t - step * CoinBurst.SparkStep, size);
                    double dot = coin.Diameter * unit * (0.42 - step * 0.08);
                    double alpha = (0.75 - step * 0.2) * (1 - flown / CoinBurst.SparkWhile);
                    canvas.Fill(CanvasShape.Circle(cx + back.Dx, cy + back.Dy, dot / 2), CoinDrawing.Spark.WithOpacity(alpha * opacity));
                }
            }

            CoinDrawing.Draw(canvas, coin with { Diameter = coin.Diameter * unit }, cx + at.Dx, cy + at.Dy, coin.Phase + coin.Spin * flown, opacity);
        }

        return canvas;
    }
}

/// <summary>
/// The tray mark's credit arrival (<c>GlassMenuBarStrip</c> "Credit arrival
/// burst", MenuBarPanel.swift:519-575): the gold gleam over the mark, its hop,
/// and ten mini coins fanned across the mark's own canvas.
/// </summary>
public static class StripBurst
{
    /// <summary>MenuBarPanel.swift:521-525.</summary>
    public const int Count = 10;
    public const double Drag = 3.8;
    public const double Gravity = 10;
    public const double HopDuration = 0.35;
    public const double HopPeak = 2.0;

    /// <summary>The gleam's rise and hold (MenuBarPanel.swift:551-552).</summary>
    public const double GleamRise = 0.15;
    public const double GleamHoldUntil = 0.7;

    /// <summary>The white-gold flash over the mark, and its strength (MenuBarPanel.swift:654-657).</summary>
    public const double MarkFlashUntil = 0.3;
    public const double MarkFlashPeak = 0.55;

    /// <summary>How long a mini coin trails its one spark (MenuBarPanel.swift:705).</summary>
    public const double SparkWhile = 0.45;

    /// <summary>MenuBarPanel.swift:529-543.</summary>
    public static IReadOnlyList<Coin> Coins { get; } = Enumerable.Range(0, Count).Select(index =>
    {
        double slot = (double)index / Math.Max(1, Count - 1) - 0.5;
        double angle = slot * 0.88 * Math.PI + (CoinBurst.Noise(index, 21) - 0.5) * 0.14;
        return new Coin(
            Angle: angle,
            Speed: 44 + CoinBurst.Noise(index, 22) * 58,
            Diameter: 2.4 + CoinBurst.Noise(index, 23) * 1.2,
            Spin: 1.8 + CoinBurst.Noise(index, 24) * 2.0,
            Phase: CoinBurst.Noise(index, 25),
            Tilt: (CoinBurst.Noise(index, 26) - 0.5) * 1.0,
            Delay: CoinBurst.Noise(index, 27) * 0.08);
    }).ToArray();

    /// <summary>MenuBarPanel.swift:546-554: how strongly the mark gleams gold.</summary>
    public static double Gleam(double t)
    {
        if (!(t >= 0) || !(t < CoinBurst.Duration))
        {
            return 0;
        }

        if (t < GleamRise)
        {
            return t / GleamRise;
        }

        if (t < GleamHoldUntil)
        {
            return 1;
        }

        return Math.Max(0, 1 - (t - GleamHoldUntil) / (CoinBurst.Duration - GleamHoldUntil));
    }

    /// <summary>MenuBarPanel.swift:558-561: the mark's rise, negative is up; none at <see cref="CoinBurst.StillAt"/>.</summary>
    public static double Hop(double t)
    {
        if (!(t > 0) || !(t < HopDuration))
        {
            return 0;
        }

        return -HopPeak * Math.Sin(Math.PI * t / HopDuration);
    }

    /// <summary>MenuBarPanel.swift:565-570: from the mark's centre.</summary>
    public static Offset OffsetOf(Coin coin, double t)
    {
        double flown = Math.Max(0, t - coin.Delay);
        double reach = coin.Speed * (1 - Math.Exp(-Drag * flown)) / Drag;
        double fall = 0.5 * Gravity * flown * flown;
        return new Offset(Math.Cos(coin.Angle) * reach, Math.Sin(coin.Angle) * reach * 0.45 + fall);
    }
}

/// <summary>Drawing one coin and a flash on a <see cref="PixelCanvas"/>, in canvas pixels.</summary>
public static class CoinDrawing
{
    public static Rgba GoldLight => Rgba.FromArgb(HomeTrayInks.CoinGoldLight);
    public static Rgba Gold => Rgba.FromArgb(HomeTrayInks.CoinGold);
    public static Rgba GoldDeep => Rgba.FromArgb(HomeTrayInks.CoinGoldDeep);
    public static Rgba GoldRim => Rgba.FromArgb(HomeTrayInks.CoinGoldRim);
    public static Rgba Spark => Rgba.FromArgb(HomeTrayInks.CoinSpark);

    /// <summary>
    /// The face's width share as it turns: a full disc face on, a sliver edge
    /// on (CoinBurst.swift:190-191).
    /// </summary>
    public static double Facing(double turn) => Math.Cos(turn * 2 * Math.PI);

    /// <summary>
    /// One coin turning end over end (<c>GlassCoinBurst.drawCoin</c>,
    /// CoinBurst.swift:188-210): its lit face a gold gradient, its back
    /// darker, a 0.6-point rim, and a white glint as it turns face on.
    /// </summary>
    public static void Draw(PixelCanvas canvas, Coin coin, double x, double y, double turn, double opacity, double pointsPerPixel = 1)
    {
        double facing = Facing(turn);
        double d = coin.Diameter;
        double width = d * Math.Max(0.2, Math.Abs(facing));
        bool lit = facing > 0;
        double cos = Math.Cos(coin.Tilt), sin = Math.Sin(coin.Tilt);

        // The gradient runs from the face's top-left corner to its bottom
        // right, in the coin's own (tilted) frame.
        double gx = width, gy = d;
        double length2 = gx * gx + gy * gy;
        Rgba Shade(double px, double py)
        {
            double lx = (px - x) * cos + (py - y) * sin + width / 2;
            double ly = -(px - x) * sin + (py - y) * cos + d / 2;
            double t = Math.Clamp((lx * gx + ly * gy) / length2, 0, 1);
            Rgba c = lit
                ? (t < 0.5 ? Rgba.Mix(GoldLight, Gold, t * 2) : Rgba.Mix(Gold, GoldDeep, (t - 0.5) * 2))
                : Rgba.Mix(Gold, GoldDeep, t);
            return c.WithOpacity(opacity);
        }

        canvas.Fill(CanvasShape.Ellipse(x, y, width / 2, d / 2, coin.Tilt), Shade);
        double rim = 0.6 / pointsPerPixel;
        canvas.Fill(
            new EllipseBand(x, y, width / 2, d / 2, coin.Tilt, rim),
            GoldRim.WithOpacity(opacity));
        if (Math.Abs(facing) > 0.92)
        {
            double glint = d * 0.32;
            // Its top-left at (-0.28 d, -0.3 d) in the coin's frame.
            double lx = -d * 0.28 + glint / 2, ly = -d * 0.3 + glint / 2;
            canvas.Fill(
                CanvasShape.Circle(x + lx * cos - ly * sin, y + lx * sin + ly * cos, glint / 2),
                new Rgba(1, 1, 1, 0.9 * opacity));
        }
    }

    /// <summary>
    /// The gold flash: a radial gradient from gold-light to clear gold, its
    /// three stops evenly spaced as SwiftUI spaces a plain colour list.
    /// </summary>
    public static void Flash(PixelCanvas canvas, double cx, double cy, double radius, double lightAlpha, double goldAlpha)
    {
        if (radius <= 0)
        {
            return;
        }

        Rgba inner = GoldLight.WithOpacity(lightAlpha);
        Rgba middle = Gold.WithOpacity(goldAlpha);
        Rgba outer = Gold.WithOpacity(0);
        canvas.Fill(CanvasShape.Circle(cx, cy, radius), (px, py) =>
        {
            double r = Math.Sqrt((px - cx) * (px - cx) + (py - cy) * (py - cy)) / radius;
            return r < 0.5 ? Rgba.Mix(inner, middle, r * 2) : Rgba.Mix(middle, outer, Math.Min(1, (r - 0.5) * 2));
        });
    }

    /// <summary>A turned ellipse's stroke, centred on its edge.</summary>
    private sealed class EllipseBand(double cx, double cy, double rx, double ry, double angle, double line) : CanvasShape
    {
        private readonly CanvasShape _outer = Ellipse(cx, cy, rx + line / 2, ry + line / 2, angle);
        private readonly CanvasShape _inner = Ellipse(cx, cy, Math.Max(0, rx - line / 2), Math.Max(0, ry - line / 2), angle);
        private readonly List<(double, double)> _a = new();
        private readonly List<(double, double)> _b = new();

        public override (double Top, double Bottom) Rows => _outer.Rows;

        public override void Spans(double y, List<(double From, double To)> spans)
        {
            _a.Clear();
            _b.Clear();
            _outer.Spans(y, _a);
            _inner.Spans(y, _b);
            foreach ((double from, double to) in _a)
            {
                if (_b.Count == 0)
                {
                    spans.Add((from, to));
                    continue;
                }

                (double cutFrom, double cutTo) = _b[0];
                if (cutFrom > from)
                {
                    spans.Add((from, Math.Min(cutFrom, to)));
                }

                if (cutTo < to)
                {
                    spans.Add((Math.Max(cutTo, from), to));
                }
            }
        }
    }
}

/// <summary>
/// Whether, and how, the Balance tile marks NEAR AI credit arriving
/// (<c>BalanceArrival</c>, macos/Sources/TCShellCore/BalanceArrival.swift).
/// Credit has arrived when the daemon's <c>received_total_nanos</c> is larger
/// than it was when the tile was last drawn, never inferred from the balance.
/// </summary>
public static class BalanceArrival
{
    /// <summary>What the tile last drew, or draws now (BalanceArrival.swift:11-24).</summary>
    public readonly record struct Reading(long? ReceivedNanos, long? RemainingNanos);

    /// <summary>BalanceArrival.swift:26-34.</summary>
    public enum EffectKind
    {
        None,
        CountUp,
        Highlight,
    }

    public readonly record struct Effect(EffectKind Kind, long FromNanos = 0, long ToNanos = 0);

    /// <summary>BalanceArrival.swift:36-45.</summary>
    public static Effect EffectOf(Reading? shown, Reading now, bool reduceMotion)
    {
        if (shown?.ReceivedNanos is not long before || now.ReceivedNanos is not long after || after <= before)
        {
            return new Effect(EffectKind.None);
        }

        if (reduceMotion)
        {
            return new Effect(EffectKind.Highlight);
        }

        if (shown?.RemainingNanos is not long from || now.RemainingNanos is not long to || from == to)
        {
            return new Effect(EffectKind.Highlight);
        }

        return new Effect(EffectKind.CountUp, from, to);
    }

    /// <summary>BalanceArrival.swift:49-57: one figure per frame, ending exactly on <paramref name="toNanos"/>.</summary>
    public static IReadOnlyList<long> CountSteps(long fromNanos, long toNanos, int frames)
    {
        if (frames <= 0)
        {
            return new[] { toNanos };
        }

        double delta = (double)toNanos - fromNanos;
        var steps = new List<long>(frames);
        for (int frame = 1; frame < frames; frame++)
        {
            steps.Add(fromNanos + (long)Math.Floor(delta * frame / frames));
        }

        steps.Add(toNanos);
        return steps;
    }

    /// <summary>The count's frames and their length (BalanceArrivalFlourish.swift:36-37).</summary>
    public const int CountFrames = 24;
    public static readonly TimeSpan CountFrame = TimeSpan.FromMilliseconds(50);

    /// <summary>How long the still coins stay under Reduce Motion (BalanceArrivalFlourish.swift:39).</summary>
    public static readonly TimeSpan Hold = TimeSpan.FromMilliseconds(1600);

    /// <summary>BalanceArrivalFlourish.swift:47-49: the coins' flight, or the still moment.</summary>
    public static TimeSpan CoinsStay(bool reduceMotion) => reduceMotion ? Hold : TimeSpan.FromSeconds(CoinBurst.Duration);

    /// <summary>
    /// Credit arrivals for the tray and taskbar animations
    /// (<c>BalanceArrival.Tracker</c>, BalanceArrival.swift): the first reading
    /// for an account is its baseline, an unknown reading keeps it, a fall
    /// resets it, and any real rise celebrates. The baseline is saved per
    /// account, so credit that arrived while the app was closed or signed out
    /// plays once on the next reading.
    /// </summary>
    public sealed class Tracker
    {
        /// <summary>The daemon sums single-precision values, so a re-read can differ in its last digits.</summary>
        public const double CreditNoiseFloor = 0.001;

        public Tracker()
        {
        }

        public Tracker(long? lastReceivedNanos, double? lastCreditFinal)
        {
            LastReceivedNanos = lastReceivedNanos;
            LastCreditFinal = lastCreditFinal;
        }

        public long? LastReceivedNanos { get; private set; }

        public double? LastCreditFinal { get; private set; }

        /// <summary>The baseline saved for <paramref name="scope"/>, or an empty tracker.</summary>
        public static Tracker Stored(string? scope, ShellPreferences preferences)
        {
            ArgumentNullException.ThrowIfNull(preferences);
            return preferences.CreditBaseline(scope) is { } saved
                ? new Tracker(saved.ReceivedNanos, saved.CreditFinal is double credit && double.IsFinite(credit) ? credit : null)
                : new Tracker();
        }

        /// <summary>Saves this baseline for <paramref name="scope"/>. Without a scope nothing is saved.</summary>
        public void Save(string? scope, ShellPreferences preferences)
        {
            ArgumentNullException.ThrowIfNull(preferences);
            preferences.SetCreditBaseline(scope, LastReceivedNanos, LastCreditFinal);
        }

        /// <summary>Records NEAR AI's received total and says whether it rose.</summary>
        public bool Observe(long? receivedTotalNanos)
        {
            if (receivedTotalNanos is not long now)
            {
                return false;
            }

            if (LastReceivedNanos is not long before)
            {
                LastReceivedNanos = now;
                return false;
            }

            if (now == before)
            {
                return false;
            }

            LastReceivedNanos = now;
            return now > before;
        }

        /// <summary>Records settled Trace credit and says whether it rose by more than rounding noise.</summary>
        public bool ObserveCreditFinal(double? creditFinal)
        {
            if (creditFinal is not double now || !double.IsFinite(now))
            {
                return false;
            }

            if (LastCreditFinal is not double before)
            {
                LastCreditFinal = now;
                return false;
            }

            if (Math.Abs(now - before) < CreditNoiseFloor)
            {
                return false;
            }

            LastCreditFinal = now;
            return now > before;
        }
    }

    /// <summary>
    /// The tray and taskbar animations' frame schedule
    /// (<c>BalanceArrival.IconAnimation</c>, BalanceArrival.swift:136-160).
    /// </summary>
    public static class IconAnimation
    {
        /// <summary>BalanceArrival.swift:149.</summary>
        public const double FramesPerSecond = 24;

        /// <summary>BalanceArrival.swift:155-157: 1000/24 rounded, 42 ms.</summary>
        public static TimeSpan FrameInterval => TimeSpan.FromMilliseconds(Math.Round(1000.0 / FramesPerSecond, MidpointRounding.AwayFromZero));

        /// <summary>BalanceArrival.swift:153: the still frame's hold.</summary>
        public static TimeSpan ReduceMotionHold => Hold;

        /// <summary>BalanceArrival.swift:160-164: the elapsed times to render for one celebration.</summary>
        public static IReadOnlyList<double> FrameTimes(bool reduceMotion)
        {
            if (reduceMotion)
            {
                return new[] { CoinBurst.StillAt };
            }

            int count = Math.Max(1, (int)Math.Ceiling(CoinBurst.Duration * FramesPerSecond));
            return Enumerable.Range(0, count).Select(i => i / FramesPerSecond).ToArray();
        }
    }
}
