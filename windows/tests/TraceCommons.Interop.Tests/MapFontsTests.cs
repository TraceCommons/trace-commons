using System;
using System.Linq;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map draws its words and its lock through DirectWrite (Win2D), which
/// reads a font family as one name: unlike XAML it never splits a
/// "First, Second" list, so a list matches nothing and falls back to a font
/// without the lock glyph. On the test box the pile's lock drew as an empty
/// box. Each family the map names must be one installed family.
/// </summary>
public class MapFontsTests
{
    private static readonly string[] Box =
    {
        "Segoe UI", "Segoe UI Variable Text", "Segoe UI Variable Display", "Segoe Fluent Icons",
        "Segoe MDL2 Assets", "Consolas", "Cascadia Mono",
    };

    [Fact]
    public void EachFamilyIsOneInstalledName()
    {
        foreach (MapFontRole role in Enum.GetValues<MapFontRole>())
        {
            string family = MapFonts.Family(role, Box);
            Assert.DoesNotContain(",", family);
            Assert.Contains(family, Box);
        }
    }

    [Fact]
    public void TheLockComesFromTheIconFontAndFallsBackToMdl2()
    {
        Assert.Equal("Segoe Fluent Icons", MapFonts.Family(MapFontRole.Icons, Box));
        // Windows 10 has no Segoe Fluent Icons; MDL2 carries the same lock (U+E72E).
        string[] windows10 = Box.Where(f => f != "Segoe Fluent Icons").ToArray();
        Assert.Equal("Segoe MDL2 Assets", MapFonts.Family(MapFontRole.Icons, windows10));
        Assert.Equal("Consolas", MapFonts.Family(MapFontRole.Mono, Box.Where(f => f != "Cascadia Mono").ToArray()));
        Assert.Equal("Segoe UI", MapFonts.Family(MapFontRole.Text, new[] { "Segoe UI" }));
    }

    [Fact]
    public void NamesMatchWhateverCaseTheSystemReports()
    {
        Assert.Equal("Segoe Fluent Icons", MapFonts.Family(MapFontRole.Icons, new[] { "segoe fluent icons" }));
    }

    [Fact]
    public void ToolsHeadingKeepsTheMacsTracking()
    {
        // MapToolPile.swift:60, `.tracking(0.8)` on the TOOLS heading.
        Assert.Equal(0.8, MapFonts.ToolsHeadingTracking);
    }
}
