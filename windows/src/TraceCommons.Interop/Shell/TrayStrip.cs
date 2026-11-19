using System;
using System.Collections.Generic;
using TraceCommons.Interop.Generated;

namespace TraceCommons.Interop.Shell;

/// <summary>What the tray mark can say about the data behind it (<c>GlassMenuBarStrip.Condition</c>, MenuBarPanel.swift:319-334).</summary>
public enum StripCondition
{
    Live,
    Paused,
    Attention,
    Unavailable,
    Loading,
}

/// <summary>The re-engagement mark the core lit (<c>GlassMenuBarStrip.Mark</c>, MenuBarPanel.swift:338-346).</summary>
public enum StripMark
{
    None,
    /// <summary>Something new to look at: a hollow ring in the badge's empty slot.</summary>
    News,
    /// <summary>Some decisions owed are idle sessions: a halo round the badge.</summary>
    Ready,
}

/// <summary>Everything the tray icon draws. Equal inputs draw the same pixels.</summary>
public readonly record struct Strip(StripCondition Condition, int? Badge, StripMark Mark)
{
    /// <summary>Before the first status reply: nothing is known yet.</summary>
    public static Strip Loading => new(StripCondition.Loading, null, StripMark.None);
}

/// <summary>The core's words for a drawn mark (<c>status.nudge.mark_text</c>).</summary>
public readonly record struct StripMarkWords(string Accessibility, string Tooltip);

/// <summary>
/// The tray icon: the Mac's menu-bar item (<c>GlassMenuBarStrip</c>,
/// macos/Sources/TCDesign/Components/MenuBarPanel.swift:297-724, and
/// <c>MenuPanelStatus</c>, Views/Monitor/MenuBarGlassPanel.swift:827-882)
/// drawn for the notification area. The NEAR AI N in the logo kit's single
/// ink, the decisions-owed badge cut out of that ink (99+ past 99, never
/// red), the news ring in the badge's empty slot on a live strip, the halo
/// round a drawn badge, the attention capsule, and the credit arrival's
/// gleam, hop and mini coins.
/// </summary>
/// <remarks>
/// <para>
/// No 24-hour bars (owner decision W12-D1 (a), 2026-11-18: mark and badge
/// only). The Mac's item without its graph is the mark then the slot in a
/// row; a tray icon is a square, so, as on Linux (#2186), the 14-point mark
/// keeps its place centred in a 22-point square and the slot moves onto the
/// square's bottom-right corner.
/// </para>
/// <para>
/// At 100% scaling the tray asks for 16 pixels, which gives the badge about
/// 8: the trade-off the owner accepted with W12-D2 (a). The count also stays
/// in the tooltip.
/// </para>
/// </remarks>
public static class TrayStrip
{
    /// <summary>MenuBarPanel.swift:475-477: the item's height, the mark's side.</summary>
    public const double Height = 22;
    public const double MarkSize = 14;

    /// <summary>MenuBarPanel.swift:460-463: the badge's height (and least width), its figure, its inset.</summary>
    public const double BadgeSize = 11;
    public const double BadgeGlyph = 7;
    public const double BadgeInset = 2.5;

    /// <summary>MenuBarPanel.swift:470-472: the halo's inset past the badge, its line, and so its reach.</summary>
    public const double HaloInset = 2.5;
    public const double HaloLine = 2;
    public const double HaloReach = HaloInset + HaloLine / 2;

    /// <summary>MenuBarPanel.swift:717-719: the news ring, 9 points stroked 1.5.</summary>
    public const double RingSize = 9;
    public const double RingLine = 1.5;

    /// <summary>MenuBarPanel.swift:514: how far the slot clears the mark.</summary>
    public const double MarkOnlyGap = 2;

    /// <summary>The badge figure's pixel face: three cells across, five down (the Mac's 7-point bold cap height).</summary>
    public const int FaceColumns = 3;
    public const int FaceRows = 5;

    /// <summary>The Mac's badge cap (MenuBarStatus.swift:31).</summary>
    public const int BadgeCap = 99;

    /// <summary>
    /// <c>MenuPanelStatus.condition</c> (MenuBarGlassPanel.swift:840-849):
    /// never live unless everything is.
    /// </summary>
    public static StripCondition Condition(int? decisionsOwed, bool unhealthy, bool paused, bool available, bool stale, bool statusAnswered)
    {
        if (!available || stale)
        {
            return StripCondition.Unavailable;
        }

        if (unhealthy)
        {
            return StripCondition.Attention;
        }

        if (!statusAnswered)
        {
            return StripCondition.Loading;
        }

        if (decisionsOwed is null)
        {
            return StripCondition.Attention;
        }

        return paused ? StripCondition.Paused : StripCondition.Live;
    }

    /// <summary><c>MenuPanelStatus.badge</c> (MenuBarGlassPanel.swift:829-832): decisions owed when there are any.</summary>
    public static int? Badge(int? decisionsOwed) => decisionsOwed is > 0 ? decisionsOwed : null;

    /// <summary>
    /// <c>NudgeSurface.mark</c> (TCShellCore/NudgeSurface.swift): only
    /// <c>news</c> and <c>ready</c>, and neither while the icon cannot vouch
    /// for what it shows. The core lights the mark only after its own mute,
    /// cooldowns and news window, so this never decides to light one.
    /// </summary>
    public static StripMark LitMark(string? nudgeMark, bool available)
    {
        if (!available)
        {
            return StripMark.None;
        }

        return nudgeMark switch
        {
            "news" => StripMark.News,
            "ready" => StripMark.Ready,
            _ => StripMark.None,
        };
    }

    /// <summary>MenuBarPanel.swift:427-430: none while unavailable or loading.</summary>
    public static int? ShownBadge(int? badge, StripCondition condition)
    {
        if (condition is StripCondition.Unavailable or StripCondition.Loading)
        {
            return null;
        }

        return badge is > 0 ? badge : null;
    }

    /// <summary>MenuBarPanel.swift:394-401: the ring only with no badge on a live strip, the halo only round a drawn badge.</summary>
    public static StripMark ShownMark(StripMark mark, int? badge, StripCondition condition)
    {
        bool badgeShown = ShownBadge(badge, condition) is not null;
        return mark switch
        {
            StripMark.News => !badgeShown && condition == StripCondition.Live ? StripMark.News : StripMark.None,
            StripMark.Ready => badgeShown ? StripMark.Ready : StripMark.None,
            _ => StripMark.None,
        };
    }

    /// <summary>MenuBarPanel.swift:434-436: two digits at most, 99+ past them.</summary>
    public static string BadgeText(int badge) => badge > BadgeCap ? $"{BadgeCap}+" : badge.ToString(System.Globalization.CultureInfo.InvariantCulture);

    /// <summary>MenuBarPanel.swift:439-441: the attention capsule.</summary>
    public static bool ShowsAttention(StripCondition condition) =>
        condition is StripCondition.Attention or StripCondition.Unavailable;

    /// <summary>
    /// <c>MenuPanelStatus.markWords</c> (MenuBarGlassPanel.swift:864-870):
    /// the core's words for the mark only while the icon draws it.
    /// </summary>
    public static StripMarkWords? MarkWords(string? nudgeMark, StripMarkWords? words, bool available, int? badge, StripCondition condition) =>
        ShownMark(LitMark(nudgeMark, available), badge, condition) == StripMark.None ? null : words;

    /// <summary>MenuBarGlassPanel.swift:874-877: the base sentence, then the drawn mark's, whole.</summary>
    public static string MarkAccessibility(string baseSentence, StripMarkWords? words) =>
        words is { Accessibility.Length: > 0 } w ? baseSentence + " " + w.Accessibility : baseSentence;

    /// <summary>
    /// The strip and the drawn mark's words from one status read
    /// (<c>MenuBarStripInputs.of</c> and <c>MenuBarStripLabel.body</c>,
    /// MenuBarGlassPanel.swift:663-675): a status that could not be read is
    /// unavailable, never the last one drawn.
    /// </summary>
    public static (Strip Strip, StripMarkWords? Words) FromStatus(DaemonStatus? status)
    {
        if (status is null)
        {
            return (new Strip(StripCondition.Unavailable, null, StripMark.None), null);
        }

        StripCondition condition = Condition(status.DecisionsOwed, Unhealthy(status), status.Paused, available: true, stale: false, statusAnswered: true);
        int? badge = Badge(status.DecisionsOwed);
        string? mark = null;
        StripMarkWords? words = null;
        if (status.Nudge is { ValueKind: System.Text.Json.JsonValueKind.Object } nudge)
        {
            if (nudge.TryGetProperty("mark", out var m) && m.ValueKind == System.Text.Json.JsonValueKind.String)
            {
                mark = m.GetString();
            }

            if (nudge.TryGetProperty("mark_text", out var t) && t.ValueKind == System.Text.Json.JsonValueKind.Object
                && t.TryGetProperty("accessibility", out var a) && a.ValueKind == System.Text.Json.JsonValueKind.String
                && t.TryGetProperty("tooltip", out var tip) && tip.ValueKind == System.Text.Json.JsonValueKind.String)
            {
                words = new StripMarkWords(a.GetString() ?? string.Empty, tip.GetString() ?? string.Empty);
            }
        }

        var strip = new Strip(condition, badge, LitMark(mark, available: true));
        return (strip, MarkWords(mark, words, available: true, badge, condition));
    }

    /// <summary>
    /// <c>model.health != nil</c> (AppModel.swift:1690-1698): a health label,
    /// unless it is <c>witness-saturated</c> while the witness card is drawn,
    /// or the gate-held label while the held-folder notice is drawn, since
    /// each of those says the same thing with more. Not every label: the
    /// Linux port read every label as unhealthy first (#2186, 0266ab54b).
    /// </summary>
    public static bool Unhealthy(DaemonStatus status)
    {
        string? label = status.Health?.LastErrorLabel;
        if (string.IsNullOrEmpty(label))
        {
            return false;
        }

        if (label == WitnessSaturatedLabel && WitnessCapacitySurface.Notice(status.WitnessCapacity) is not null)
        {
            return false;
        }

        if (label == SwitchOnNotices.GateHeldLabel && SwitchOnNotices.Held(status.AutomaticContributionHeld) is not null)
        {
            return false;
        }

        return true;
    }

    /// <summary>The daemon's label for a busy privacy witness (<c>daemon::health::LABEL_WITNESS_SATURATED</c>).</summary>
    public const string WitnessSaturatedLabel = "witness-saturated";

    /// <summary>The pixel face of a badge character, rows top to bottom, a set bit per lit cell from the left.</summary>
    internal static int[]? Face(char c) => c switch
    {
        '0' => new[] { 0b111, 0b101, 0b101, 0b101, 0b111 },
        '1' => new[] { 0b010, 0b110, 0b010, 0b010, 0b111 },
        '2' => new[] { 0b111, 0b001, 0b111, 0b100, 0b111 },
        '3' => new[] { 0b111, 0b001, 0b111, 0b001, 0b111 },
        '4' => new[] { 0b101, 0b101, 0b111, 0b001, 0b001 },
        '5' => new[] { 0b111, 0b100, 0b111, 0b001, 0b111 },
        '6' => new[] { 0b111, 0b100, 0b111, 0b101, 0b111 },
        '7' => new[] { 0b111, 0b001, 0b010, 0b010, 0b010 },
        '8' => new[] { 0b111, 0b101, 0b111, 0b101, 0b111 },
        '9' => new[] { 0b111, 0b101, 0b111, 0b001, 0b111 },
        '+' => new[] { 0b000, 0b010, 0b111, 0b010, 0b000 },
        '!' => new[] { 0b010, 0b010, 0b010, 0b000, 0b010 },
        _ => null,
    };

    /// <summary>The figure's width in cells: three per character, one between them.</summary>
    internal static int FaceWidth(string text) => text.Length == 0 ? 0 : text.Length * FaceColumns + (text.Length - 1);

    /// <summary>The badge's width for <paramref name="text"/>: the figure and its inset each side, never narrower than tall.</summary>
    public static double BadgeWidth(string text, double cell) => Math.Max(FaceWidth(text) * cell + 2 * BadgeInset, BadgeSize);

    /// <summary>The N's outline, flattened, in the unit square (aspect kept), built once.</summary>
    private static readonly Lazy<IReadOnlyList<IReadOnlyList<(double X, double Y)>>> UnitOutline = new(() => Flatten(NearMarkPath.Unit, 16));

    /// <summary>Cubic segments become this many lines each: at 32 pixels a segment spans a few pixels at most.</summary>
    internal static IReadOnlyList<IReadOnlyList<(double X, double Y)>> Flatten(IReadOnlyList<PathStep> steps, int perCubic)
    {
        var rings = new List<IReadOnlyList<(double, double)>>();
        List<(double, double)>? ring = null;
        double px = 0, py = 0;
        foreach (PathStep s in steps)
        {
            switch (s.Kind)
            {
                case PathStepKind.Move:
                    if (ring is { Count: > 2 })
                    {
                        rings.Add(ring);
                    }

                    ring = new List<(double, double)> { (s.X, s.Y) };
                    break;
                case PathStepKind.Line:
                    (ring ??= new List<(double, double)>()).Add((s.X, s.Y));
                    break;
                case PathStepKind.Cubic:
                    ring ??= new List<(double, double)>();
                    for (int n = 1; n <= perCubic; n++)
                    {
                        double t = (double)n / perCubic, u = 1 - t;
                        ring.Add((
                            u * u * u * px + 3 * u * u * t * s.X1 + 3 * u * t * t * s.X2 + t * t * t * s.X,
                            u * u * u * py + 3 * u * u * t * s.Y1 + 3 * u * t * t * s.Y2 + t * t * t * s.Y));
                    }

                    break;
                case PathStepKind.Close:
                    if (ring is { Count: > 2 })
                    {
                        rings.Add(ring);
                    }

                    ring = null;
                    break;
            }

            px = s.X;
            py = s.Y;
        }

        if (ring is { Count: > 2 })
        {
            rings.Add(ring);
        }

        return rings;
    }

    /// <summary>The N, <paramref name="size"/> across with its top-left at (x, y), in canvas pixels (<c>GlassBrandMark.path(at:size:)</c>).</summary>
    public static CanvasShape MarkShape(double x, double y, double size)
    {
        var rings = new List<IReadOnlyList<(double, double)>>();
        foreach (IReadOnlyList<(double X, double Y)> unit in UnitOutline.Value)
        {
            var ring = new (double, double)[unit.Count];
            for (int i = 0; i < unit.Count; i++)
            {
                ring[i] = (x + unit[i].X * size, y + unit[i].Y * size);
            }

            rings.Add(ring);
        }

        return CanvasShape.Polygons(rings);
    }

    /// <summary>The slot's contents, in points of the 22-point square.</summary>
    public readonly record struct Slot((double X, double Y, double W, double H)? Badge, bool Halo, (double X, double Y)? Ring, (double X, double Y, double W, double H)? Attention);

    /// <summary>Where the slot's contents sit for <paramref name="strip"/>, a figure cell of <paramref name="cell"/> points.</summary>
    public static Slot SlotOf(Strip strip, double cell)
    {
        StripMark shown = ShownMark(strip.Mark, strip.Badge, strip.Condition);
        if (ShownBadge(strip.Badge, strip.Condition) is int n)
        {
            double w = BadgeWidth(BadgeText(n), cell);
            return new Slot((Height - w, Height - BadgeSize, w, BadgeSize), shown == StripMark.Ready, null, null);
        }

        if (ShowsAttention(strip.Condition))
        {
            // MenuBarPanel.swift:705-712: "!" on a capsule at least badgeSize, in the badge's place.
            double w = BadgeWidth("!", cell);
            return new Slot(null, false, null, (Height - w, Height - BadgeSize, w, BadgeSize));
        }

        if (shown == StripMark.News)
        {
            double outer = RingSize / 2 + RingLine / 2;
            return new Slot(null, false, (Height - outer, Height - outer), null);
        }

        return new Slot(null, false, null, null);
    }

    /// <summary>The inks for one taskbar appearance.</summary>
    public readonly record struct Inks(Rgba Mark, Rgba Accent, Rgba Attention, Rgba OnAttention)
    {
        public static Inks For(bool darkTaskbar) => darkTaskbar
            ? new(Rgba.FromArgb(HomeTrayInks.Dark.MenuItemMark), Rgba.FromArgb(HomeTrayInks.Dark.PurpleText), Rgba.FromArgb(HomeTrayInks.Dark.MenuModeNever), Rgba.FromArgb(HomeTrayInks.HeroInk))
            : new(Rgba.FromArgb(HomeTrayInks.Light.MenuItemMark), Rgba.FromArgb(HomeTrayInks.Light.PurpleText), Rgba.FromArgb(HomeTrayInks.Light.MenuModeNever), Rgba.FromArgb(HomeTrayInks.HeroInk));
    }

    /// <summary>
    /// Draws <paramref name="strip"/> into a square <paramref name="px"/>
    /// pixels across, <paramref name="burst"/> seconds into a credit arrival
    /// (or at rest for null).
    /// </summary>
    public static PixelCanvas Render(Strip strip, bool darkTaskbar, int px, double? burst = null)
    {
        var canvas = new PixelCanvas(px, px);
        double unit = px / Height;

        // The figure's cell: one point, snapped to whole pixels so the face is crisp.
        double cellPx = Math.Max(1, Math.Round(unit));
        double cell = cellPx / unit;
        Slot slot = SlotOf(strip, cell);
        Inks inks = Inks.For(darkTaskbar);
        double inset = (Height - MarkSize) / 2;

        if (burst is double t && t >= 0 && t < CoinBurst.Duration)
        {
            DrawBurst(canvas, unit, inset, t, inks.Mark);
        }
        else
        {
            canvas.Fill(MarkShape(inset * unit, inset * unit, MarkSize * unit), inks.Mark);
        }

        // Clear the mark under the slot with the gap to spare: the Mac's
        // markOnlyGap, plus the halo's reach while a halo rings the badge.
        if (slot.Badge is { } b)
        {
            double o = slot.Halo ? MarkOnlyGap + HaloReach : MarkOnlyGap;
            canvas.Clear(CanvasShape.Capsule((b.X - o) * unit, (b.Y - o) * unit, (b.W + 2 * o) * unit, (b.H + 2 * o) * unit));
        }

        if (slot.Attention is { } a)
        {
            double o = MarkOnlyGap;
            canvas.Clear(CanvasShape.Capsule((a.X - o) * unit, (a.Y - o) * unit, (a.W + 2 * o) * unit, (a.H + 2 * o) * unit));
        }

        if (slot.Ring is { } r)
        {
            canvas.Clear(CanvasShape.Circle(r.X * unit, r.Y * unit, (RingSize / 2 + RingLine / 2 + MarkOnlyGap) * unit));
        }

        if (slot.Badge is { } badge && ShownBadge(strip.Badge, strip.Condition) is int count)
        {
            canvas.Fill(CanvasShape.Capsule(badge.X * unit, badge.Y * unit, badge.W * unit, badge.H * unit), inks.Mark);

            // The figure, cut out of the badge (MenuBarPanel.swift:690-693, destinationOut).
            CutFace(canvas, BadgeText(count), badge, unit, cell);
            if (slot.Halo)
            {
                canvas.Fill(
                    CanvasShape.CapsuleStroke(
                        (badge.X - HaloInset) * unit,
                        (badge.Y - HaloInset) * unit,
                        (badge.W + 2 * HaloInset) * unit,
                        (badge.H + 2 * HaloInset) * unit,
                        HaloLine * unit),
                    inks.Accent);
            }
        }

        if (slot.Attention is { } capsule)
        {
            canvas.Fill(CanvasShape.Capsule(capsule.X * unit, capsule.Y * unit, capsule.W * unit, capsule.H * unit), inks.Attention);
            FillFace(canvas, "!", capsule, unit, cell, inks.OnAttention);
        }

        if (slot.Ring is { } ring)
        {
            canvas.Fill(CanvasShape.Ring(ring.X * unit, ring.Y * unit, RingSize / 2 * unit, RingLine * unit), inks.Accent);
        }

        return canvas;
    }

    private static IEnumerable<(double X, double Y)> FaceCells(string text, (double X, double Y, double W, double H) box, double unit, double cell)
    {
        double fw = FaceWidth(text) * cell;

        // Whole pixels, so every cell lands on the device grid.
        double left = Math.Round((box.X + (box.W - fw) / 2) * unit) / unit;
        double top = Math.Round((box.Y + (box.H - FaceRows * cell) / 2) * unit) / unit;
        for (int i = 0; i < text.Length; i++)
        {
            if (Face(text[i]) is not int[] rows)
            {
                continue;
            }

            double cx = left + i * (FaceColumns + 1) * cell;
            for (int row = 0; row < FaceRows; row++)
            {
                for (int col = 0; col < FaceColumns; col++)
                {
                    if ((rows[row] & (1 << (FaceColumns - 1 - col))) != 0)
                    {
                        yield return (cx + col * cell, top + row * cell);
                    }
                }
            }
        }
    }

    private static void CutFace(PixelCanvas canvas, string text, (double X, double Y, double W, double H) box, double unit, double cell)
    {
        foreach ((double x, double y) in FaceCells(text, box, unit, cell))
        {
            canvas.Clear(CanvasShape.Rect(x * unit, y * unit, cell * unit, cell * unit));
        }
    }

    private static void FillFace(PixelCanvas canvas, string text, (double X, double Y, double W, double H) box, double unit, double cell, Rgba ink)
    {
        foreach ((double x, double y) in FaceCells(text, box, unit, cell))
        {
            canvas.Fill(CanvasShape.Rect(x * unit, y * unit, cell * unit, cell * unit), ink);
        }
    }

    /// <summary>
    /// The mark's canvas <paramref name="t"/> seconds into a credit arrival,
    /// as MenuBarPanel.swift:633-711 draws it: the flash, the mark lifted by
    /// the hop with the gold gleam and the white-gold flash over it, then the
    /// mini coins with their one spark each. The Mac's canvas without its
    /// graph is the mark's own 14-point column; here that is the mark's
    /// column of the square, and no coin leaves it.
    /// </summary>
    private static void DrawBurst(PixelCanvas canvas, double unit, double inset, double t, Rgba ink)
    {
        double width = MarkSize, height = Height;
        double cx = (inset + MarkSize / 2) * unit, cy = height / 2 * unit;
        if (t < CoinBurst.FlashDuration)
        {
            double left = 1 - t / CoinBurst.FlashDuration;
            double radius = height / 2 * (0.45 + 0.55 * (t / CoinBurst.FlashDuration));
            CoinDrawing.Flash(canvas, cx, cy, radius * unit, 0.85 * left, 0.35 * left);
        }

        double originY = (height - MarkSize) / 2 + StripBurst.Hop(t);
        CanvasShape logo = MarkShape(inset * unit, originY * unit, MarkSize * unit);
        canvas.Fill(logo, ink);
        double gleam = StripBurst.Gleam(t);
        if (gleam > 0)
        {
            canvas.Fill(logo, CoinDrawing.Gold.WithOpacity(gleam));
        }

        if (t < StripBurst.MarkFlashUntil)
        {
            canvas.Fill(logo, CoinDrawing.GoldLight.WithOpacity((1 - t / StripBurst.MarkFlashUntil) * StripBurst.MarkFlashPeak));
        }

        double opacity = CoinBurst.Opacity(t);
        foreach (Coin coin in StripBurst.Coins)
        {
            double flown = t - coin.Delay;
            if (!(flown > 0))
            {
                continue;
            }

            Offset d = StripBurst.OffsetOf(coin, t);
            double x = MarkSize / 2 + d.Dx, y = height / 2 + d.Dy;
            if (x < 1 || x > width - 1 || y < 1 || y > height - 1)
            {
                continue;
            }

            if (flown < StripBurst.SparkWhile)
            {
                Offset back = StripBurst.OffsetOf(coin, Math.Max(0, t - 0.04));
                double dot = coin.Diameter * 0.38;
                canvas.Fill(
                    CanvasShape.Circle((inset + MarkSize / 2 + back.Dx) * unit, (height / 2 + back.Dy) * unit, dot / 2 * unit),
                    CoinDrawing.Spark.WithOpacity(0.75 * (1 - flown / StripBurst.SparkWhile) * opacity));
            }

            CoinDrawing.Draw(
                canvas,
                coin with { Diameter = coin.Diameter * unit },
                (inset + x) * unit,
                y * unit,
                coin.Phase + coin.Spin * flown,
                opacity,
                1 / unit);
        }
    }
}
