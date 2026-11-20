using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The tile in a tree row (<c>GlassToolTile</c>,
/// macos/Sources/TCDesign/Components/Indicators.swift:247-334, sizes
/// macos/Sources/TCDesign/Generated/GlassTokens.swift:479-481 and :577-580).
/// Every expected value is a Swift literal at the line cited, flat theme.
/// </summary>
public class ToolTileTests
{
    private static ToolTileFace Face(ToolTileKind kind, bool large = false, string? mark = "dir") =>
        ToolTile.Face(kind, large, mark);

    [Fact]
    public void ASmallTileIs22RoundedAt6AndALargeOne30RoundedAt11()
    {
        // GlassTokens.swift:577-578 toolTile 22, toolTileLarge 30;
        // :481 Radius.tile 6; :479 Radius.control flat 11 (Indicators.swift:282-284).
        foreach (ToolTileKind kind in new ToolTileKind[]
        {
            new ToolTileKind.Tool("codex"), ToolTileKind.Folder, new ToolTileKind.FolderInitial("app"), ToolTileKind.Add, ToolTileKind.Session,
        })
        {
            ToolTileFace small = Face(kind);
            Assert.Equal(22, small.Side);
            Assert.Equal(6, small.CornerRadius);
            ToolTileFace large = Face(kind, large: true);
            Assert.Equal(30, large.Side);
            Assert.Equal(11, large.CornerRadius);
        }
    }

    [Theory]
    [InlineData("claude-code", "claude", "toolClaude")]
    [InlineData("codex", "codex", "toolCodex")]
    [InlineData("opencode", "openCode", "toolOpenCode")]
    public void AToolWithALogoDrawsItInItsTintOnTheInkAtATenth(string tool, string logo, string tint)
    {
        // Indicators.swift:287-302: logo 15 (large 20, GlassTokens.swift:579-580)
        // filled in tool.tint, on ink(0.1); tints :209-219, logos :222-233.
        ToolTileFace face = Face(new ToolTileKind.Tool(tool));
        Assert.Equal(ToolTileMarkKind.Logo, face.Mark);
        Assert.Equal(logo, face.Text);
        Assert.Equal(tint, face.InkToken);
        Assert.Equal(15, face.MarkSize);
        Assert.Equal(20, Face(new ToolTileKind.Tool(tool), large: true).MarkSize);
        Assert.Equal("ink", face.GroundToken);
        Assert.Equal(0.1, face.GroundAlpha);
    }

    [Theory]
    [InlineData("gemini-cli", "Gm", "toolAntigravity")]
    [InlineData("cline", "Cl", "textPrimary")]
    public void AToolWithNoLogoDrawsItsInitialsBold9(string tool, string initials, string tint)
    {
        // Indicators.swift:294-297: glassGlyph(large ? 11 : 9, weight: .bold);
        // initials :236-245, tint :209-219.
        ToolTileFace face = Face(new ToolTileKind.Tool(tool));
        Assert.Equal(ToolTileMarkKind.Text, face.Mark);
        Assert.Equal(initials, face.Text);
        Assert.Equal(tint, face.InkToken);
        Assert.Equal(9, face.MarkSize);
        Assert.Equal(ToolTileWeight.Bold, face.Weight);
        Assert.Equal(11, Face(new ToolTileKind.Tool(tool), large: true).MarkSize);
        Assert.Equal("ink", face.GroundToken);
        Assert.Equal(0.1, face.GroundAlpha);
    }

    [Fact]
    public void AFolderIsTheFolderGlyphInPrimaryOnTheFolderFill()
    {
        // Indicators.swift:305-310: folder.fill, glassGlyph(large ? 13 : 11),
        // regular, textPrimary, on tileFolder.
        ToolTileFace face = Face(ToolTileKind.Folder);
        Assert.Equal(ToolTileMarkKind.Glyph, face.Mark);
        Assert.Equal(ToolTile.FolderGlyph, face.Text);
        Assert.Equal(11, face.MarkSize);
        Assert.Equal(13, Face(ToolTileKind.Folder, large: true).MarkSize);
        Assert.Equal(ToolTileWeight.Regular, face.Weight);
        Assert.Equal("textPrimary", face.InkToken);
        Assert.Equal("tileFolder", face.GroundToken);
        Assert.Equal(1, face.GroundAlpha);
    }

    [Fact]
    public void AFolderInitialIsItsFirstLetterUpperCasedInTheFolderInk()
    {
        // Indicators.swift:311-316 (glassGlyph(large ? 11 : 9, .bold),
        // tileFolderInk on tileFolder) and :262-265 (first letter upper-cased,
        // the folder mark for none).
        ToolTileFace face = Face(new ToolTileKind.FolderInitial("app"));
        Assert.Equal(ToolTileMarkKind.Text, face.Mark);
        Assert.Equal("A", face.Text);
        Assert.Equal(9, face.MarkSize);
        Assert.Equal(11, Face(new ToolTileKind.FolderInitial("app"), large: true).MarkSize);
        Assert.Equal(ToolTileWeight.Bold, face.Weight);
        Assert.Equal("tileFolderInk", face.InkToken);
        Assert.Equal("tileFolder", face.GroundToken);
        Assert.Equal("dir", Face(new ToolTileKind.FolderInitial(string.Empty)).Text);
        Assert.Equal(string.Empty, Face(new ToolTileKind.FolderInitial(string.Empty), mark: null).Text);
        Assert.Equal("É", ToolTile.Initial("école", "dir"));
        Assert.Equal("dir", ToolTile.Initial(string.Empty, "dir"));
    }

    [Fact]
    public void AnAddBoxIsAPlusInSecondarySemiboldOnTheInkAtATenth()
    {
        // Indicators.swift:317-322: plus, glassGlyph(large ? 13 : 10, .semibold), textSecondary, ink(0.1).
        ToolTileFace face = Face(ToolTileKind.Add);
        Assert.Equal(ToolTileMarkKind.Glyph, face.Mark);
        Assert.Equal(ToolTile.AddGlyph, face.Text);
        Assert.Equal(10, face.MarkSize);
        Assert.Equal(13, Face(ToolTileKind.Add, large: true).MarkSize);
        Assert.Equal(ToolTileWeight.Semibold, face.Weight);
        Assert.Equal("textSecondary", face.InkToken);
        Assert.Equal("ink", face.GroundToken);
        Assert.Equal(0.1, face.GroundAlpha);
    }

    [Fact]
    public void ASessionIsTheBoxMarkBold11InStatusOffAtBothSizes()
    {
        // Indicators.swift:279 sessionMark "▤"; :323-328 glassGlyph(11, .bold), statusOff, tileFolder.
        ToolTileFace face = Face(ToolTileKind.Session);
        Assert.Equal(ToolTileMarkKind.Text, face.Mark);
        Assert.Equal("▤", face.Text);
        Assert.Equal(11, face.MarkSize);
        Assert.Equal(11, Face(ToolTileKind.Session, large: true).MarkSize);
        Assert.Equal(ToolTileWeight.Bold, face.Weight);
        Assert.Equal("statusOff", face.InkToken);
        Assert.Equal("tileFolder", face.GroundToken);
    }

    [Fact]
    public void ASessionTakesItsDeclaredToolFirstThenItsAdaptersThenNone()
    {
        // TracesViews.swift:933-941: SourceKind(rawValue: declaredSource ?? source)
        // ?? SourceKind(rawValue: source), else the plain session tile.
        Assert.Equal(new ToolTileKind.Tool("codex"), ToolTile.ForSession("claude-code", "codex"));
        Assert.Equal(new ToolTileKind.Tool("claude-code"), ToolTile.ForSession("claude-code", null));
        Assert.Equal(new ToolTileKind.Tool("claude-code"), ToolTile.ForSession("claude-code", "someone-else"));
        Assert.Equal(ToolTileKind.Session, ToolTile.ForSession("someone-else", null));
        Assert.Equal(ToolTileKind.Session, ToolTile.ForSession("someone-else", "other"));
    }
}
