using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Mac's glyphs on Windows (spec W5-D2 (a)): its own paths, exported
/// from Glyphs.swift, and one table of Segoe icons for its SF Symbols.
/// </summary>
public class GlassGlyphTests
{
    // Glyphs.swift:9, the cases in order, and :31-32 / :57-60 for two of them.
    [Fact]
    public void TheGlyphsAreTheMacsPaths()
    {
        Assert.Equal(
            new[] { "viewMenu", "graph", "map", "inspector", "gear", "chevronDown", "folder", "kebab" },
            GlassGlyphs.All.Select(g => g.Name));
        Assert.Equal("M1,2h12M1,6h12M1,10h12", GlassGlyphs.ViewMenu.Data);
        Assert.Equal((14.0, 12.0, 1.4), (GlassGlyphs.ViewMenu.ViewWidth, GlassGlyphs.ViewMenu.ViewHeight, GlassGlyphs.ViewMenu.Stroke));
        // The graph and inspector icons share the rounded frame (Glyphs.swift:24-25).
        Assert.StartsWith("M2.5,0.5H11.5A2,2,0,0,1,13.5,2.5V9.5", GlassGlyphs.Graph.Data);
        Assert.StartsWith("M2.5,0.5H11.5A2,2,0,0,1,13.5,2.5V9.5", GlassGlyphs.Inspector.Data);
        // The kebab is filled, not stroked.
        Assert.Equal(0, GlassGlyphs.Kebab.Stroke);
    }

    /// <summary>
    /// Every SF Symbol the Mac names, outside its gallery: the 35 literal
    /// names (<c>grep -rhoE '(systemName|systemImage): *"[a-z0-9.]+"'</c> over
    /// macos/Sources less TCDesign/Gallery, at 841d81ba5) and the six it names
    /// through a constant.
    /// </summary>
    [Fact]
    public void EveryMacSymbolHasAGlyph()
    {
        string[] literal =
        {
            "arrow.right", "bell", "binoculars", "checklist", "checkmark", "chevron.left", "chevron.right",
            "circle.fill", "clock", "creditcard", "doc.text", "dollarsign.circle", "exclamationmark.circle.fill",
            "folder.fill", "gearshape", "hand.thumbsup", "hand.thumbsup.fill", "info.circle", "lightbulb", "lock",
            "lock.fill", "lock.shield", "minus", "nosign", "person.crop.circle.badge.checkmark", "plus", "power",
            "shield.lefthalf.filled", "sparkles", "star.fill", "terminal", "testtube.2", "tray",
            "tray.and.arrow.down", "xmark",
        };
        string[] named = { "arrow.clockwise", "chevron.down", "chevron.up", "exclamationmark.triangle.fill", "magnifyingglass", "line.3.horizontal.decrease" };
        Assert.Equal(35, literal.Length);
        Assert.Equal(literal.Concat(named).OrderBy(s => s, StringComparer.Ordinal), SymbolGlyphs.Table.Keys.OrderBy(s => s, StringComparer.Ordinal));
        Assert.All(SymbolGlyphs.Table.Values, g => Assert.InRange(g.CodePoint, 0xE000, 0xF8FF));
    }

    /// <summary>
    /// On Windows, every code point is in the installed icon font: Segoe
    /// Fluent Icons, or Segoe MDL2 Assets where the Fluent font is absent
    /// (Windows 10, some Server editions). A glyph neither font has would draw
    /// as an empty box.
    /// </summary>
    [WindowsFact]
    public void EveryGlyphIsInTheInstalledIconFont()
    {
        string fonts = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "Fonts");
        var present = new List<(string Font, HashSet<int> Cmap)>();
        foreach (string file in new[] { "SegoeIcons.ttf", "segmdl2.ttf" })
        {
            string path = Path.Combine(fonts, file);
            if (File.Exists(path))
            {
                present.Add((file, TrueTypeCmap.CodePoints(File.ReadAllBytes(path))));
            }
        }

        Assert.True(present.Count > 0, "neither SegoeIcons.ttf nor segmdl2.ttf is installed");
        string first = present[0].Font;
        var missing = SymbolGlyphs.Table
            .Where(kv => !present[0].Cmap.Contains(kv.Value.CodePoint))
            .Select(kv => $"{kv.Key} U+{kv.Value.CodePoint:X4}")
            .ToList();
        Assert.True(missing.Count == 0, $"not in {first}: {string.Join(", ", missing)}");
    }
}

/// <summary>Reads the Unicode code points a TrueType font maps (its cmap, formats 4 and 12).</summary>
internal static class TrueTypeCmap
{
    public static HashSet<int> CodePoints(byte[] font)
    {
        int U16(int o) => (font[o] << 8) | font[o + 1];
        long U32(int o) => ((long)font[o] << 24) | ((long)font[o + 1] << 16) | ((long)font[o + 2] << 8) | font[o + 3];
        int tables = U16(4);
        int cmap = -1;
        for (int i = 0; i < tables; i++)
        {
            int record = 12 + i * 16;
            if (font[record] == 'c' && font[record + 1] == 'm' && font[record + 2] == 'a' && font[record + 3] == 'p')
            {
                cmap = (int)U32(record + 8);
            }
        }

        Assert.True(cmap >= 0, "the font has a cmap");
        var points = new HashSet<int>();
        int subtables = U16(cmap + 2);
        for (int i = 0; i < subtables; i++)
        {
            int at = cmap + (int)U32(cmap + 4 + i * 8 + 4);
            int format = U16(at);
            if (format == 4)
            {
                int segments = U16(at + 6) / 2;
                int ends = at + 14, starts = ends + segments * 2 + 2;
                for (int s = 0; s < segments; s++)
                {
                    int end = U16(ends + s * 2), start = U16(starts + s * 2);
                    if (start == 0xFFFF)
                    {
                        continue;
                    }

                    int deltaAt = starts + segments * 2, rangeAt = deltaAt + segments * 2;
                    int delta = U16(deltaAt + s * 2), range = U16(rangeAt + s * 2);
                    for (int c = start; c <= end; c++)
                    {
                        int glyph;
                        if (range == 0)
                        {
                            glyph = (c + delta) & 0xFFFF;
                        }
                        else
                        {
                            int index = rangeAt + s * 2 + range + (c - start) * 2;
                            glyph = U16(index);
                            if (glyph != 0)
                            {
                                glyph = (glyph + delta) & 0xFFFF;
                            }
                        }

                        if (glyph != 0)
                        {
                            points.Add(c);
                        }
                    }
                }
            }
            else if (format == 12)
            {
                long groups = U32(at + 12);
                for (int g = 0; g < groups; g++)
                {
                    int group = at + 16 + g * 12;
                    long start = U32(group), end = U32(group + 4);
                    for (long c = start; c <= end; c++)
                    {
                        points.Add((int)c);
                    }
                }
            }
        }

        return points;
    }
}
