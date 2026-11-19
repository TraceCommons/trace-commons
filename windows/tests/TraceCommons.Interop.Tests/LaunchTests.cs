using System.Collections.Generic;
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

    [Theory]
    [InlineData(@"C:\Users\x\proj", true)]
    [InlineData(@"C:proj", false)]
    [InlineData(@"proj", false)]
    public void TheSavedAccountLaunchSendsOnlyAFullyQualifiedFolder(string folder, bool sent)
    {
        // The sibling launch path (checklist L4): the saved-account Launch
        // dialog's typed folder takes the same rule as the sign-in start.
        var account = new ManagedAccount("a", "claude", "subscription", "Work", "ready");
        var request = Snapshot().AccountLaunchParams(account, folder, model: null, collectTraces: false, "r");
        Assert.Equal(sent, request is not null);
        if (request is not null)
        {
            Assert.Equal(folder, request["cwd"]);
            Assert.Equal("a", request["account_id"]);
            Assert.Equal("coding", request["purpose"]);
            Assert.Equal(false, request["save_default"]);
            Assert.Null(request["model"]);
            Assert.Equal(false, request["collect_traces"]);
        }
    }

    [Fact]
    public void AnOpenCodeAccountLaunchCarriesItsModelAndCollection()
    {
        var account = new ManagedAccount("o", "opencode", "api_key", "Keys", "ready");
        var request = Snapshot().AccountLaunchParams(account, @"C:\p", model: " provider/model ", collectTraces: true, "r")!;
        Assert.Equal("provider/model", request["model"]);
        Assert.Equal(true, request["collect_traces"]);
        Assert.Null(Snapshot().AccountLaunchParams(account, @"C:\p", model: "  ", collectTraces: false, "r"));
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
    public void AnUnloadedRaisedWhileTheArtIsStillShownLeavesTheMoveRunning()
    {
        // Box, 2026-11-19: a ContentDialog raises Unloaded on its content
        // about 340 ms after Loaded while the dialog stays open, so resting
        // on every Unloaded stopped the move before its first visible frame.
        var life = new LaunchArtLife();
        Assert.Equal(LaunchArtStep.Animate, life.Loaded(animationsEnabled: true));
        Assert.Equal(LaunchArtStep.None, life.Unloaded(stillLoaded: true));
        Assert.False(life.Resting);
        Assert.True(life.Watching);
        // A real Unloaded, once the art has left the tree, releases it and
        // draws nothing: its visual is gone (box, 2026-11-19: stopping its
        // animation then threw and crashed the app).
        Assert.Equal(LaunchArtStep.None, life.Unloaded(stillLoaded: false));
        Assert.True(life.Resting);
        Assert.False(life.Watching);
    }

    [Fact]
    public void ASettingChangeThatTurnsAnimationsOffRestsTheArt()
    {
        // Box, 2026-11-19: turning animations off mid-move raised no
        // UISettings.AnimationsEnabledChanged while the window kept the
        // foreground, and the folder slid on; WM_SETTINGCHANGE does arrive.
        var life = new LaunchArtLife();
        life.Loaded(animationsEnabled: true);
        Assert.Equal(LaunchArtStep.None, life.SettingChanged(animationsEnabled: true));
        Assert.False(life.Resting);
        Assert.Equal(LaunchArtStep.Rest, life.SettingChanged(animationsEnabled: false));
        Assert.True(life.Resting);
        // Once at rest, nothing more happens.
        Assert.Equal(LaunchArtStep.None, life.SettingChanged(animationsEnabled: false));
    }

    [Fact]
    public void ClosingTheDialogReleasesTheArtWithNoSecondUnloaded()
    {
        // Review of #2207: the late Unloaded returns early, so an art whose
        // dialog raised no second one stayed watched for the app's life.
        // The dialog's close releases it; the watch list is then empty.
        var watched = new LaunchArtWatchList<object>();
        var art = new object();
        var life = new LaunchArtLife();
        life.Loaded(animationsEnabled: true);
        if (life.Watching) watched.Add(art);
        life.Unloaded(stillLoaded: true);
        if (!life.Watching) watched.Remove(art);
        Assert.Equal(1, watched.Count);

        // The close touches no visual: the art has left the tree.
        Assert.Equal(LaunchArtStep.None, life.Closed());
        Assert.False(life.Watching);
        if (!life.Watching) watched.Remove(art);
        Assert.Equal(0, watched.Count);
        // A setting change after the release does nothing, and a late
        // Unloaded after the close does not rest it twice.
        Assert.Equal(LaunchArtStep.None, life.SettingChanged(animationsEnabled: false));
        Assert.Equal(LaunchArtStep.None, life.Unloaded(stillLoaded: false));
    }

    [Fact]
    public void TheWatchListHoldsEachArtOnceAndIsSafeToChangeWhileRead()
    {
        var watched = new LaunchArtWatchList<object>();
        var a = new object();
        var b = new object();
        watched.Add(a);
        watched.Add(a);
        watched.Add(b);
        Assert.Equal(2, watched.Count);
        foreach (object art in watched.Snapshot()) watched.Remove(art);
        Assert.Equal(0, watched.Count);
    }

    [Fact]
    public void WithAnimationsOffTheArtIsItsRestFrameFromTheStart()
    {
        // ManagedFolderExplainer.swift:187, :218-219: Reduce Motion draws the rest frame.
        Assert.True(LaunchHandoff.StartsAtRest(animationsEnabled: false));
        var life = new LaunchArtLife();
        Assert.Equal(LaunchArtStep.Rest, life.Loaded(animationsEnabled: false));
        Assert.True(life.Resting);
        Assert.True(life.Watching);
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
        // Review of #2207: these four survived a mutation run unpinned.
        // Radius.control under the flat theme (GlassTokens.swift:480).
        Assert.Equal(11.0, LaunchHandoff.ControlRadius);
        // ink(0.06) ground, ink(0.18) dots (ManagedFolderExplainer.swift:231, :235).
        Assert.Equal((0.06, 0.18), (LaunchHandoff.TerminalInk, LaunchHandoff.DotInk));
        // HStack(spacing: 4) of 6 pt circles, padded 9, bar 18, hairline 1 (:233-241).
        Assert.Equal((6.0, 4.0, 9.0, 18.0, 1.0),
            (LaunchHandoff.DotSide, LaunchHandoff.DotSpacing, LaunchHandoff.BarPadding, LaunchHandoff.BarHeight, LaunchHandoff.Hairline));
        // The large tool tile (GlassTokens.swift:578; Indicators.swift:282-284).
        Assert.Equal(30.0, LaunchHandoff.FolderTileSide);
        // The launch button: logo 14, spaced s3 = 6 (:152, :166; GlassTokens.swift:504).
        Assert.Equal((14.0, 6.0), (LaunchHandoff.ButtonLogoSide, LaunchHandoff.ButtonSpacing));
    }

    [Fact]
    public void UnderHighContrastTheTerminalIsAnOutlineWithFullInkDots()
    {
        // Box, 2026-11-19, High Contrast Black: the 0.06 ground drew solid,
        // hiding the logo and dots drawn in the same system colour.
        Assert.Equal((0.06, 0.18), LaunchHandoff.TerminalInks(highContrast: false));
        Assert.Equal((0.0, 1.0), LaunchHandoff.TerminalInks(highContrast: true));
    }

    [Fact]
    public void TheFolderIsTheFilledFolderGlyph()
    {
        // Indicators.swift:306 `folder.fill`: Segoe Fluent Icons FolderFill,
        // not the outline Folder (U+E8B7) the box diff showed.
        Assert.Equal("\uE8D5", LaunchHandoff.FolderGlyph);
    }

    // ---- The card's foot, failure and focus -------------------------------

    [Fact]
    public void InstallIsAskedOnlyWhenNoLaunchedToolIsInstalled()
    {
        // ManagedSessionsSurface.swift:990-1007: needsToolInstallation and
        // missingInstallTools key on installation, not on launchability.
        Assert.False(Snapshot().NeedsToolInstallation);
        Assert.Empty(Snapshot().MissingInstallTools);
        var some = Snapshot("""
            [{"tool":"opencode","installed":false,"install_url":"c"},{"tool":"claude","installed":true,"install_url":"a"},{"tool":"codex","installed":false,"install_url":"b"}]
            """);
        Assert.False(some.NeedsToolInstallation);
        Assert.Equal(ManagedToolArea.Buttons, some.ToolArea);
        // In the shell's order, whatever the daemon's.
        Assert.Equal(["codex", "opencode"], some.MissingInstallTools.Select(t => t.Tool).ToArray());
        var none = Snapshot("""
            [{"tool":"claude","installed":false,"install_url":"a"},{"tool":"red_team","installed":true,"install_url":"r"}]
            """);
        Assert.True(none.NeedsToolInstallation);
        Assert.Equal(["claude"], none.MissingInstallTools.Select(t => t.Tool).ToArray());
        // A row with no launched tool at all asks for an install, as the Mac's does.
        var noLaunched = Snapshot("""[{"tool":"red_team","installed":true,"install_url":"r"}]""");
        Assert.True(noLaunched.NeedsToolInstallation);
        Assert.Equal(ManagedToolArea.Install, noLaunched.ToolArea);
    }

    [Fact]
    public void ACardLaunchsFailureIsSaidOnTheCardAndClearedByTheNextAction()
    {
        // ManagedSessionsView.swift:131-136, :160-163, :215-228; checklist L3.
        var copy = new Dictionary<string, string>
        {
            ["action_failed"] = "That didn't work.",
            ["terminal_not_found"] = "{terminal} wasn't found on this computer. Choose another in Settings.",
            ["terminal"] = "your terminal",
        };
        string T(string key) => copy.GetValueOrDefault(key, string.Empty);
        var failure = new ManagedFailure();
        Assert.Empty(failure.CardLines(T, "Windows Terminal"));
        Assert.False(failure.OnCard(cardShown: true));

        failure.Refuse("terminal_not_found", fromCard: true);
        Assert.True(failure.OnCard(cardShown: true));
        Assert.False(failure.OnCard(cardShown: false));
        Assert.Equal(new[] { "That didn't work.", "Windows Terminal wasn't found on this computer. Choose another in Settings." },
            failure.CardLines(T, "Windows Terminal"));
        Assert.Equal("your terminal wasn't found on this computer. Choose another in Settings.", failure.CardLines(T, null)[1]);

        // The next action clears it.
        failure.Begin();
        Assert.Null(failure.Key);
        Assert.False(failure.OnCard(cardShown: true));
        Assert.Empty(failure.CardLines(T, "Windows Terminal"));

        // "That didn't work." alone is said once; a refusal from elsewhere is not the card's.
        failure.Refuse("action_failed", fromCard: true);
        Assert.Equal(new[] { "That didn't work." }, failure.CardLines(T, null));
        failure.Refuse("action_failed", fromCard: false);
        Assert.False(failure.OnCard(cardShown: true));
    }

    [Fact]
    public void FocusReturnsToThePressedButtonOnceTheDialogCloses()
    {
        // Box, 2026-11-19: closing the launch dialog dropped focus to the
        // Settings gear, because every read rebuilds the view.
        string[] names = ["Start Claude Code", "Start Codex", "Remove", "Remove"];
        Assert.Equal(("Remove", 1), ManagedFocus.KeyOf(names, 3));
        Assert.Equal(("Start Codex", 0), ManagedFocus.KeyOf(names, 1));
        Assert.Null(ManagedFocus.KeyOf(names, -1));
        Assert.Equal(3, ManagedFocus.Find(names, ("Remove", 1)));
        Assert.Equal(-1, ManagedFocus.Find(names, ("Remove", 2)));
        Assert.Equal(-1, ManagedFocus.Find(names, ("Gone", 0)));

        (string, int) pressed = ("Start Codex", 0);
        // A poll while the dialog is open: nothing on the page has focus, and
        // the return is kept.
        var (target, keep) = ManagedFocus.Rebuild(null, busy: true, pressed);
        Assert.Null(target);
        Assert.Equal(pressed, keep);
        // A second poll, still open: kept again.
        (target, keep) = ManagedFocus.Rebuild(null, busy: true, keep);
        Assert.Equal(pressed, keep);
        // The dialog has closed: focus goes back to the pressed button, once.
        (target, keep) = ManagedFocus.Rebuild(null, busy: false, keep);
        Assert.Equal(pressed, target);
        Assert.Null(keep);
        // A button that has focus keeps it.
        (target, _) = ManagedFocus.Rebuild(("Remove", 1), busy: false, pressed);
        Assert.Equal(("Remove", 1), target);
    }
}
