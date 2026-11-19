using System.Linq;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// W14 (launch): the per-tool Start buttons, the launch dialog's optional
/// folder, the tool logos and the folder-into-terminal art.
/// </summary>
/// <remarks>
/// Numbers marked "probe" were printed by the throwaway probe that compiles
/// macos/Sources/TCDesign/Logos/SVGPathData.swift and GlassLogoPaths.swift at
/// 409066936 with a main that walks each logo's CGPath (element counts,
/// <c>boundingBoxOfPath</c>) and evaluates SwiftUI's
/// <c>UnitCurve.bezier(0.2, 0.8, 0.2, 1)</c> (GlassMotion.swift:7-12,
/// GlassTokens.swift:698-701) at the handoff's times. Everything else is a
/// Swift literal cited by file and line.
/// </remarks>
public sealed class LaunchTests
{
    // ---- Start buttons ---------------------------------------------------

    private const string SignedIn = """
    {"revision":1,"accounts":[],"sign_in_accounts":[{"id":"c","tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"x","tool":"codex","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"o","tool":"opencode","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}],"sessions":[],"generations":{"claude":4},"capabilities":{"managed_launch":true,"near_ai_signin":true,"terminal_launch":true},"copy":{}}
    """;

    private static ManagedSnapshot Snapshot(string? tools = null) =>
        ManagedSnapshot.Parse(tools is null ? SignedIn : SignedIn.Replace("\"copy\":{}", "\"copy\":{},\"tools\":" + tools))!;

    [Fact]
    public void OpenCodeGetsAStartButtonOnlyWhenTheDaemonSaysItIsInstalled()
    {
        // ManagedSessionsSurface.swift:1087-1098: OpenCode only while the
        // `tools` row says installed; an older daemon with no `tools` offers
        // none (fail closed), Claude Code and Codex keep the older reading.
        Assert.Equal(["claude", "codex"], Snapshot().SignInStartTools);
        Assert.Equal(["claude", "codex", "opencode"], Snapshot("""
            [{"tool":"claude","installed":true,"install_url":"a"},{"tool":"codex","installed":true,"install_url":"b"},{"tool":"opencode","installed":true,"install_url":"c"}]
            """).SignInStartTools);
        Assert.Equal(["claude"], Snapshot("""
            [{"tool":"claude","installed":true,"install_url":"a"},{"tool":"codex","installed":false,"install_url":"b"},{"tool":"opencode","installed":false,"install_url":"c"}]
            """).SignInStartTools);
    }

    [Fact]
    public void EachStartToolHasTheMacsStartKey()
    {
        // ManagedSessionsSurface.swift:327-329.
        Assert.Equal(new[] { "start_claude", "start_codex", "start_opencode" },
            ManagedSignIn.StartTools.Select(t => ManagedSignIn.StartKey(t)!).ToArray());
        Assert.Equal("start_red_team", ManagedSignIn.StartKey("red_team"));
        Assert.Null(ManagedSignIn.StartKey("future_tool"));
    }

    [Fact]
    public void TheCardsFootFollowsTheMacsOrder()
    {
        // ManagedSessionsView.swift:125-129: no terminal first, then install.
        var snapshot = Snapshot();
        Assert.Equal(ManagedToolArea.Buttons, snapshot.ToolArea);
        Assert.Equal(ManagedToolArea.NoTerminal,
            (snapshot with { Capabilities = snapshot.Capabilities with { TerminalLaunch = false } }).ToolArea);
        Assert.Equal(ManagedToolArea.Install, Snapshot("""
            [{"tool":"claude","installed":false,"install_url":"a"},{"tool":"codex","installed":false,"install_url":"b"},{"tool":"opencode","installed":false,"install_url":"c"}]
            """).ToolArea);
    }

    // ---- The launch dialog's folder -------------------------------------

    [Fact]
    public void NoFolderNamesNoCwdAndTheDaemonUsesTheHomeFolder()
    {
        // ManagedSessionsSurface.swift:1104-1114: `if let cwd { params["cwd"] = cwd }`.
        var request = Snapshot().SignInLaunchParams("claude", null, "r");
        Assert.NotNull(request);
        Assert.False(request.ContainsKey("cwd"));
        Assert.Equal("c", request["account_id"]);
        Assert.Equal(@"C:\p", Snapshot().SignInLaunchParams("claude", @"C:\p", "r")!["cwd"]);
    }

    [Fact]
    public void AStartIsOnlyForAToolTheCardOffers()
    {
        // ManagedSessionsSurface.swift:1105: `guard signInStartTools.contains(tool)`.
        Assert.Null(Snapshot().SignInLaunchParams("opencode", null, "r"));
        Assert.Null(Snapshot().SignInLaunchParams("future_tool", null, "r"));
    }

    [Theory]
    [InlineData(@"C:\Users\x\My Projects\app", true)]
    [InlineData(@"C:\", true)]
    [InlineData(@"\\server\share\dir", true)]
    [InlineData(@"C:proj", false)]
    [InlineData(@"\proj", false)]
    [InlineData(@"proj\sub", false)]
    [InlineData(@"..\up", false)]
    [InlineData(@" C:\lead", false)]
    [InlineData("", false)]
    [InlineData(null, false)]
    public void ALaunchSendsOnlyAFullyQualifiedFolder(string? folder, bool sent)
    {
        Assert.Equal(sent, ManagedLaunchFolder.IsAcceptable(folder));
        if (folder is not null and not "")
        {
            // The wiring: the request builder refuses what the rule refuses.
            Assert.Equal(sent, Snapshot().SignInLaunchParams("claude", folder, "r") is not null);
        }
    }

    [Fact]
    public void TheOpenButtonNamesTheTool()
    {
        // ManagedFolderExplainer.swift:53-55; copy.rs `open_tool`.
        Assert.Equal("Open Claude Code", ManagedSignIn.OpenLabel("Open {tool}", "Claude Code"));
    }

    // ---- Tool logos ------------------------------------------------------

    [Theory]
    // probe: claude: move 1 line 157 curve 1 close 1 bounds 0 0 24 24
    [InlineData(ToolLogoId.Claude, 1, 157, 1, 1, 0.0, 0.0, 24.0, 24.0)]
    // probe: codex: move 3 line 9 curve 42 close 3 bounds 0.00028635 -0.00125035 24.00026776 24.00135225
    [InlineData(ToolLogoId.Codex, 3, 9, 42, 3, 0.00028635315670133923, -0.0012503548664261751, 24.000267763002473, 24.001352251216595)]
    // probe: openCode: move 2 line 8 curve 0 close 2 bounds 4 2 16 20
    [InlineData(ToolLogoId.OpenCode, 2, 8, 0, 2, 4.0, 2.0, 16.0, 20.0)]
    public void EachLogoParsesToTheMacsPath(ToolLogoId id, int moves, int lines, int curves, int closes,
        double x, double y, double width, double height)
    {
        var steps = ToolLogos.Steps(id);
        Assert.Equal(moves, steps.Count(s => s.Kind == PathStepKind.Move));
        Assert.Equal(lines, steps.Count(s => s.Kind == PathStepKind.Line));
        Assert.Equal(curves, steps.Count(s => s.Kind == PathStepKind.Cubic));
        Assert.Equal(closes, steps.Count(s => s.Kind == PathStepKind.Close));
        LayoutRect b = NearMarkPath.Bounds(steps);
        Assert.Equal(x, b.X, 3);
        Assert.Equal(y, b.Y, 3);
        Assert.Equal(width, b.Width, 3);
        Assert.Equal(height, b.Height, 3);
    }

    [Fact]
    public void CompactNumbersAndBareArcFlagsReadAsTheMacReadsThem()
    {
        // SVGPathData.swift:7-8 and :203-209: `.08-.23` is two numbers and
        // `0 00-.975 0` two flags then -.975.
        var steps = SvgPathData.Parse("M1,1l.08-.23a1,1,0,00-.975,0z")!;
        Assert.Equal(1.08, steps[1].X, 9);
        Assert.Equal(0.77, steps[1].Y, 9);
        Assert.Equal(1.08 - 0.975, steps[2].X, 9);
        Assert.Equal(PathStepKind.Close, steps[^1].Kind);
        Assert.Null(SvgPathData.Parse("1,1L2,2"));
        Assert.Null(SvgPathData.Parse("M1,1L2"));
    }

    [Fact]
    public void ALogoIsFittedCentredWithItsAspectKept()
    {
        // GlassToolLogo.swift:43-50: OpenCode's 16x20 mark in its 24 viewBox,
        // into a 14 pt square (ManagedFolderExplainer.swift:166).
        LayoutRect b = NearMarkPath.Bounds(ToolLogos.Fitted(ToolLogoId.OpenCode, new LayoutRect(0, 0, 14, 14)));
        Assert.Equal(4 * 14 / 24.0, b.X, 9);
        Assert.Equal(2 * 14 / 24.0, b.Y, 9);
        Assert.Equal(16 * 14 / 24.0, b.Width, 9);
    }

    [Theory]
    // ManagedSessionsView.swift:479-486, Indicators.swift:183-238.
    [InlineData("claude", ToolLogoId.Claude, "TcToolClaudeBrush", "CC")]
    [InlineData("codex", ToolLogoId.Codex, "TcToolCodexBrush", "Cx")]
    [InlineData("opencode", ToolLogoId.OpenCode, "TcToolOpenCodeBrush", "OC")]
    [InlineData("red_team", null, "TextFillColorPrimaryBrush", "Re")]
    public void EachToolDrawsTheMacsMark(string tool, ToolLogoId? logo, string tint, string initials)
    {
        Assert.Equal(new LaunchToolMark(logo, tint, initials), LaunchToolMark.For(tool));
    }

    // ---- The folder-into-terminal art ------------------------------------

    [Theory]
    // probe: handoff t=0.0s progress 0.0 folderX -112.0 scale 1.0
    [InlineData(0.0, -112.0, 1.0)]
    // probe: t=0.12s progress 0.39669227 folderX -81.05800247 scale 0.96033077
    [InlineData(0.12, -81.05800247192383, 0.9603307723999024)]
    // probe: t=0.3s progress 0.76728439 folderX -52.15181732 scale 0.92327156
    [InlineData(0.3, -52.151817321777344, 0.9232715606689453)]
    // probe: t=0.6s progress 0.94607925 folderX -38.20581817 scale 0.90539207
    [InlineData(0.6, -38.20581817626953, 0.905392074584961)]
    // probe: t=1.08s progress 0.99882984 folderX -34.09127235 scale 0.90011701
    [InlineData(1.08, -34.09127235412598, 0.900117015838623)]
    // probe: t=1.2s progress 1.0 folderX -34.0 scale 0.9
    [InlineData(1.2, -34.0, 0.9)]
    [InlineData(5.0, -34.0, 0.9)]
    public void TheFolderSlidesInOnTheTokenCurveOverOnePointTwoSeconds(double elapsed, double x, double scale)
    {
        var folder = LaunchHandoff.Folder(elapsed, animationsEnabled: true);
        // SwiftUI's UnitCurve evaluates in Float: agreement to 1e-3 pt.
        Assert.Equal(x, folder.X, 2);
        Assert.Equal(scale, folder.Scale, 4);
        Assert.Equal(9, folder.Y);
    }

    [Fact]
    public void TheFolderMovesRightIntoTheTerminal()
    {
        // Direction: -112 (outside the terminal's left edge) to -34, never back.
        double last = double.MinValue;
        for (int n = 0; n <= 60; n++)
        {
            double x = LaunchHandoff.Folder(n * LaunchHandoff.Duration / 60, true).X;
            Assert.True(x >= last, $"frame {n}: {x} < {last}");
            last = x;
        }

        Assert.True(LaunchHandoff.FolderStartX < -LaunchHandoff.TerminalWidth / 2);
        Assert.True(LaunchHandoff.FolderRestX > -LaunchHandoff.TerminalWidth / 2);
    }

    [Fact]
    public void WithAnimationsOffTheArtIsItsRestFrameFromTheStart()
    {
        // ManagedFolderExplainer.swift:187, :218-219: Reduce Motion draws the rest frame.
        Assert.True(LaunchHandoff.StartsAtRest(animationsEnabled: false));
        Assert.Equal((-34.0, 9.0, 0.9), LaunchHandoff.Folder(0, animationsEnabled: false));
    }

    [Fact]
    public void TheArtHasTheMacsGeometry()
    {
        // ManagedFolderExplainer.swift:198-205, :233-247.
        Assert.Equal((260.0, 108.0), (LaunchHandoff.Width, LaunchHandoff.Height));
        Assert.Equal((156.0, 100.0), (LaunchHandoff.TerminalWidth, LaunchHandoff.TerminalHeight));
        Assert.Equal((34.0, 45.0), LaunchHandoff.LogoOffset);
        Assert.Equal(28.0, LaunchHandoff.LogoSide);
    }
}
