using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Map;

/// <summary>What a run of map text is set in.</summary>
public enum MapFontRole
{
    /// <summary>Labels and headings.</summary>
    Text,

    /// <summary>Counts and paths.</summary>
    Mono,

    /// <summary>Glyphs: the coin's lock.</summary>
    Icons,
}

/// <summary>
/// The map's font families, each resolved to ONE installed family name.
/// The map draws through DirectWrite (Win2D), which takes a family as a
/// single name and never splits a XAML-style "First, Second" list: a list
/// names no family and falls back to a font without the icon glyphs.
/// </summary>
public static class MapFonts
{
    /// <summary>The TOOLS heading's letter spacing, in points (MapToolPile.swift:60, <c>.tracking(0.8)</c>).</summary>
    public const double ToolsHeadingTracking = 0.8;

    /// <summary>Preferred families, best first; the last is the one every supported Windows has.</summary>
    public static IReadOnlyList<string> Preferred(MapFontRole role) => role switch
    {
        MapFontRole.Icons => new[] { "Segoe Fluent Icons", "Segoe MDL2 Assets" },
        MapFontRole.Mono => new[] { "Cascadia Mono", "Consolas" },
        _ => new[] { "Segoe UI Variable Text", "Segoe UI" },
    };

    /// <summary>The first preferred family that is installed, else the last preferred one.</summary>
    public static string Family(MapFontRole role, IEnumerable<string> installed)
    {
        var have = new HashSet<string>(installed, StringComparer.OrdinalIgnoreCase);
        IReadOnlyList<string> preferred = Preferred(role);
        foreach (string family in preferred)
        {
            if (have.Contains(family))
            {
                return family;
            }
        }

        return preferred[preferred.Count - 1];
    }
}
