using System;
using System.Globalization;

namespace TraceCommons.Interop.Map;

/// <summary>
/// What a tile in a tree row shows (<c>GlassToolTile.Kind</c>,
/// Indicators.swift:252-260): a tool's mark, a folder, a folder marked with
/// its name's first letter, the first run's "add your tool" box, or a
/// session of a tool this build does not know.
/// </summary>
public abstract record ToolTileKind
{
    private ToolTileKind()
    {
    }

    /// <summary>A tool, by its adapter id (<c>claude-code</c>, <c>codex</c>, ...).</summary>
    public sealed record Tool(string ToolId) : ToolTileKind
    {
        /// <summary>How the glass system draws the tool (<c>SourceKind.glassTool</c>).</summary>
        public GlassToolArt Art => MapTools.Art(ToolId);
    }

    /// <summary>A folder: the folder glyph.</summary>
    public sealed record FolderTile : ToolTileKind;

    /// <summary>A folder marked with its name's first letter (#1146 <c>HistoryRow</c>).</summary>
    public sealed record FolderInitial(string Name) : ToolTileKind;

    /// <summary>The "+" of an add box.</summary>
    public sealed record AddTile : ToolTileKind;

    /// <summary>A session of a tool this build does not know.</summary>
    public sealed record SessionTile : ToolTileKind;

    public static readonly ToolTileKind Folder = new FolderTile();

    public static readonly ToolTileKind Add = new AddTile();

    public static readonly ToolTileKind Session = new SessionTile();
}

/// <summary>What a tile's mark is drawn with.</summary>
public enum ToolTileMarkKind
{
    /// <summary>A logo's path (<see cref="MapArt.Logos"/>), filled in the ink.</summary>
    Logo,

    /// <summary>Text: a tool's initials, a folder's letter or the session mark.</summary>
    Text,

    /// <summary>A system icon glyph (the Mac's SF Symbol).</summary>
    Glyph,
}

/// <summary>A glyph's weight (<c>GlassWeight</c>).</summary>
public enum ToolTileWeight
{
    Regular,
    Semibold,
    Bold,
}

/// <summary>
/// Everything a tile draws: its square, its rounding, its ground, and its
/// mark, each a Mac value (<c>GlassToolTile.body</c>, Indicators.swift:281-330).
/// </summary>
/// <param name="Side">The square's side, 22 or 30 (GlassTokens.swift:577-578).</param>
/// <param name="CornerRadius">Radius.tile, or Radius.control for a large tile (flat theme).</param>
/// <param name="GroundToken">The glass token the square is filled with.</param>
/// <param name="GroundAlpha">Its alpha: <c>ink(0.1)</c> is the ink at 0.1.</param>
/// <param name="Mark">What the mark is drawn with.</param>
/// <param name="MarkSize">The logo's frame, or the glyph's point size.</param>
/// <param name="InkToken">The glass token the mark is drawn in.</param>
/// <param name="Text">The logo's name, the text, or the glyph's name (<see cref="ToolTile.FolderGlyph"/>, <see cref="ToolTile.AddGlyph"/>).</param>
/// <param name="Weight">The text's or glyph's weight.</param>
public sealed record ToolTileFace(
    double Side,
    double CornerRadius,
    string GroundToken,
    double GroundAlpha,
    ToolTileMarkKind Mark,
    double MarkSize,
    string InkToken,
    string Text,
    ToolTileWeight Weight);

/// <summary>The Mac's <c>GlassToolTile</c> (Indicators.swift:247-334), as values.</summary>
public static class ToolTile
{
    /// <summary><c>GlassTokens.Size.toolTile</c> (GlassTokens.swift:577).</summary>
    public const double Side = 22;

    /// <summary><c>GlassTokens.Size.toolTileLarge</c> (GlassTokens.swift:578).</summary>
    public const double SideLarge = 30;

    /// <summary><c>GlassTokens.Size.toolLogo</c> (GlassTokens.swift:579).</summary>
    public const double Logo = 15;

    /// <summary><c>GlassTokens.Size.toolLogoLarge</c> (GlassTokens.swift:580).</summary>
    public const double LogoLarge = 20;

    /// <summary><c>GlassTokens.Radius.tile</c> (GlassTokens.swift:481).</summary>
    public const double Radius = 6;

    /// <summary><c>GlassTokens.Radius.control</c> in the flat theme (GlassTokens.swift:479).</summary>
    public const double RadiusLarge = 11;

    /// <summary>The session tile's mark, as #1146 draws it (<c>sessionMark</c>, Indicators.swift:279).</summary>
    public const string SessionMark = "▤";

    /// <summary>The Mac's <c>folder.fill</c>.</summary>
    public const string FolderGlyph = "folder.fill";

    /// <summary>The Mac's <c>plus</c>.</summary>
    public const string AddGlyph = "plus";

    /// <summary>
    /// A name's first letter, upper-cased; the folder mark for none
    /// (<c>initial(_:mark:)</c>, Indicators.swift:262-265).
    /// </summary>
    public static string Initial(string name, string mark)
    {
        if (string.IsNullOrEmpty(name))
        {
            return mark;
        }

        // Swift's `first` is a whole grapheme cluster; `uppercased()` takes no locale.
        string first = StringInfo.GetNextTextElement(name, 0);
        return first.ToUpperInvariant();
    }

    /// <summary>
    /// A session row's tile: its tool's, the declared source first, then the
    /// adapter's; the plain session tile for a source this build does not
    /// know (<c>sessionTile</c> and <c>tool</c>, TracesViews.swift:933-941).
    /// </summary>
    public static ToolTileKind ForSession(string source, string? declaredSource)
    {
        string? kind = MapTools.Known(declaredSource ?? source) ? declaredSource ?? source
            : MapTools.Known(source) ? source : null;
        return kind is null ? ToolTileKind.Session : new ToolTileKind.Tool(kind);
    }

    /// <summary>
    /// What a tile draws. <paramref name="folderMark"/> is the core's folder
    /// mark (<c>glassFolderMark</c>); none is drawn without it.
    /// </summary>
    public static ToolTileFace Face(ToolTileKind kind, bool large, string? folderMark)
    {
        double side = large ? SideLarge : Side;
        // The large tile rounds like a control (#1146 `.tc-tool-tile--lg`).
        double radius = large ? RadiusLarge : Radius;
        switch (kind)
        {
            case ToolTileKind.Tool tool:
                GlassToolArt art = tool.Art;
                if (art.Logo is { } logo)
                {
                    return new ToolTileFace(side, radius, "ink", 0.1, ToolTileMarkKind.Logo,
                        large ? LogoLarge : Logo, art.TintToken, logo, ToolTileWeight.Regular);
                }

                return new ToolTileFace(side, radius, "ink", 0.1, ToolTileMarkKind.Text,
                    large ? 11 : 9, art.TintToken, art.Initials, ToolTileWeight.Bold);
            case ToolTileKind.FolderTile:
                return new ToolTileFace(side, radius, "tileFolder", 1, ToolTileMarkKind.Glyph,
                    large ? 13 : 11, "textPrimary", FolderGlyph, ToolTileWeight.Regular);
            case ToolTileKind.FolderInitial initial:
                return new ToolTileFace(side, radius, "tileFolder", 1, ToolTileMarkKind.Text,
                    large ? 11 : 9, "tileFolderInk", Initial(initial.Name, folderMark ?? string.Empty), ToolTileWeight.Bold);
            case ToolTileKind.AddTile:
                return new ToolTileFace(side, radius, "ink", 0.1, ToolTileMarkKind.Glyph,
                    large ? 13 : 10, "textSecondary", AddGlyph, ToolTileWeight.Semibold);
            case ToolTileKind.SessionTile:
                return new ToolTileFace(side, radius, "tileFolder", 1, ToolTileMarkKind.Text,
                    11, "statusOff", SessionMark, ToolTileWeight.Bold);
            default:
                throw new ArgumentOutOfRangeException(nameof(kind));
        }
    }
}
