using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>One logo: path data and the viewBox it is drawn in (GlassLogoPaths.swift:15-18).</summary>
public sealed record ToolLogoArtwork(LayoutRect ViewBox, string[] Paths);

/// <summary>Which logo, tint and initials a tool id draws with.</summary>
public enum ToolLogoId
{
    Claude,
    Codex,
    OpenCode,
}

/// <summary>
/// W14 (launch): a tool's mark on its launch button and in the launch art,
/// as the Mac's <c>GlassTool</c> (Indicators.swift:179-240) for the tools a
/// launch starts, picked by <c>ManagedWords.glassTool</c>
/// (ManagedSessionsView.swift:479-486).
/// </summary>
/// <remarks>
/// The W3 slice owns the tool tiles and may move this into its own tile
/// control; the logo data is generated (<c>ToolLogoPaths.g.cs</c>) from the
/// Mac's one copy, so either reads the same paths.
/// </remarks>
public sealed record LaunchToolMark(ToolLogoId? Logo, string TintKey, string Initials)
{
    /// <summary>ManagedSessionsView.swift:479-486 then Indicators.swift:183-238.</summary>
    public static LaunchToolMark For(string tool) => tool switch
    {
        "claude" => new(ToolLogoId.Claude, "TcToolClaudeBrush", "CC"),
        "codex" => new(ToolLogoId.Codex, "TcToolCodexBrush", "Cx"),
        "opencode" => new(ToolLogoId.OpenCode, "TcToolOpenCodeBrush", "OC"),
        // `.other(initials: String(tool.prefix(2)).capitalized)`, drawn in
        // textPrimary (Indicators.swift:187).
        _ => new(null, "TextFillColorPrimaryBrush", Capitalized(tool.Length > 2 ? tool[..2] : tool)),
    };

    private static string Capitalized(string s) =>
        s.Length == 0 ? s : char.ToUpperInvariant(s[0]) + s[1..].ToLowerInvariant();

    /// <summary>The logo's paths, parsed once; null for a tool with none.</summary>
    public IReadOnlyList<PathStep>? Steps => Logo is ToolLogoId id ? ToolLogos.Steps(id) : null;
}

/// <summary>The parsed tool logos and the fit the Mac's <c>GlassToolLogoShape</c> draws them at.</summary>
public static class ToolLogos
{
    private static readonly Dictionary<ToolLogoId, IReadOnlyList<PathStep>> Parsed = new();

    public static ToolLogoArtwork Artwork(ToolLogoId id) => id switch
    {
        ToolLogoId.Claude => ToolLogoPaths.Claude,
        ToolLogoId.Codex => ToolLogoPaths.Codex,
        ToolLogoId.OpenCode => ToolLogoPaths.OpenCode,
        _ => throw new ArgumentOutOfRangeException(nameof(id)),
    };

    /// <summary>Every path of the logo, joined, in viewBox units (GlassToolLogo.swift:24-31).</summary>
    public static IReadOnlyList<PathStep> Steps(ToolLogoId id)
    {
        lock (Parsed)
        {
            if (Parsed.TryGetValue(id, out var steps))
            {
                return steps;
            }

            var all = new List<PathStep>();
            foreach (string data in Artwork(id).Paths)
            {
                all.AddRange(SvgPathData.Parse(data) ?? throw new FormatException($"logo {id} does not parse"));
            }

            Parsed[id] = all;
            return all;
        }
    }

    /// <summary>
    /// The logo fitted into <paramref name="rect"/>, aspect kept and centred
    /// (<c>GlassToolLogoShape.path(in:)</c>, GlassToolLogo.swift:43-50).
    /// </summary>
    public static IReadOnlyList<PathStep> Fitted(ToolLogoId id, LayoutRect rect)
    {
        LayoutRect box = Artwork(id).ViewBox;
        double scale = Math.Min(rect.Width / box.Width, rect.Height / box.Height);
        double midX = rect.X + rect.Width / 2, midY = rect.Y + rect.Height / 2;
        double boxMidX = box.X + box.Width / 2, boxMidY = box.Y + box.Height / 2;
        double Tx(double x) => midX + (x - boxMidX) * scale;
        double Ty(double y) => midY + (y - boxMidY) * scale;
        var steps = Steps(id);
        var fitted = new PathStep[steps.Count];
        for (int k = 0; k < steps.Count; k++)
        {
            PathStep s = steps[k];
            fitted[k] = s with { X1 = Tx(s.X1), Y1 = Ty(s.Y1), X2 = Tx(s.X2), Y2 = Ty(s.Y2), X = Tx(s.X), Y = Ty(s.Y) };
        }

        return fitted;
    }
}

/// <summary>
/// The launch dialog's art: a folder slides into a terminal window that
/// shows the tool's logo, once, then rests (<c>ManagedFolderHandoff</c>,
/// ManagedFolderExplainer.swift:177-252). Numbers are the Mac's, cited by
/// line; positions are offsets from the art's centre, as SwiftUI's
/// <c>.offset</c> in a centred <c>ZStack</c>.
/// </summary>
public static class LaunchHandoff
{
    /// <summary>The ruling's duration, on the token curve (:184, :221).</summary>
    public const double Duration = 1.2;

    /// <summary>The art's frame (:198-199).</summary>
    public const double Width = 260, Height = 108;

    /// <summary>The terminal window (:200-201).</summary>
    public const double TerminalWidth = 156, TerminalHeight = 100, TerminalX = 0;

    /// <summary>The folder's and the logo's vertical offset (:202).</summary>
    public const double ContentY = 9;

    /// <summary>Where the folder starts, outside the terminal's left edge (:203).</summary>
    public const double FolderStartX = -112;

    /// <summary>Where it rests, inside the terminal left of the logo (:204).</summary>
    public const double FolderRestX = TerminalX - 34;

    /// <summary>The logo's offset in the terminal (:205).</summary>
    public const double LogoX = TerminalX + 34;

    /// <summary>The folder's scale at rest (:211).</summary>
    public const double RestScale = 0.9;

    /// <summary>The logo's side (:246), and the title bar: dots 6 apart 4, padded 9, bar 18, hairline 1 (:233-241).</summary>
    public const double LogoSide = 28, DotSide = 6, DotSpacing = 4, BarPadding = 9, BarHeight = 18, Hairline = 1;

    /// <summary>The terminal's and the large tile's corner: Radius.control under the flat theme (GlassTokens.swift:480).</summary>
    public const double ControlRadius = 11;

    /// <summary>The terminal's ground and dots: ink at 0.06 and 0.18 (:231, :235).</summary>
    public const double TerminalInk = 0.06, DotInk = 0.18;

    /// <summary>A launch button: logo 14, spaced s3 = 6 from the name (:152, :166; GlassTokens.swift:504).</summary>
    public const double ButtonLogoSide = 14, ButtonSpacing = 6;

    /// <summary>The folder tile is the large tool tile: 30, radius control (Indicators.swift:282-284; GlassTokens.swift:578).</summary>
    public const double FolderTileSide = 30;

    /// <summary>Under Reduce Motion (animations off) the art starts on its rest frame (:187, :218-219).</summary>
    public static bool StartsAtRest(bool animationsEnabled) => !animationsEnabled;

    /// <summary>
    /// Whether an Unloaded event puts the art at rest. WinUI can raise
    /// Unloaded late, after the element has moved into a dialog's popup and
    /// while it is still shown (box, 2026-11-19: 340 ms after Loaded, with
    /// no second Loaded); only an art that is no longer loaded rests.
    /// </summary>
    public static bool RestsOnUnloaded(bool stillLoaded) => !stillLoaded;

    /// <summary>The folder at <paramref name="elapsed"/> seconds from the art appearing.</summary>
    public static (double X, double Y, double Scale) Folder(double elapsed, bool animationsEnabled)
    {
        double p = StartsAtRest(animationsEnabled) ? 1 : MonitorMotion.Ease(Math.Clamp(elapsed / Duration, 0, 1));
        return (FolderStartX + (FolderRestX - FolderStartX) * p, ContentY, 1 + (RestScale - 1) * p);
    }

    /// <summary>The logo's top-left offset from the terminal's top centre: x LogoX, y H/2 + ContentY - 14 (:247).</summary>
    public static (double X, double Y) LogoOffset => (LogoX - TerminalX, TerminalHeight / 2 + ContentY - 14);
}
