using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Monitor window's pure rules against the Mac's: tabs and their keys,
/// the breadcrumb, inspector demand, the notice stack, the motion curve and
/// the window's growth at the screen edge.
/// </summary>
/// <remarks>
/// Expected values are the Mac's, printed by the throwaway probe that compiles
/// macos/Sources at 3372425f4 (TCDesign whole; MonitorShell.swift:43-66,
/// MonitorWindowView.swift:24-92 and :652-679, MonitorNavigation.swift:1-30
/// extracted verbatim), or Swift literals cited by file and line.
/// </remarks>
public class MonitorShellTests
{
    private static readonly MonitorTab[] DefaultOrder =
        { MonitorTab.Home, MonitorTab.Inference, MonitorTab.Traces, MonitorTab.Insights };

    // ---- Tabs ------------------------------------------------------------

    [Theory]
    // probe: ordered('') / ordered('Insights,Traces') / ordered('Home,Bogus,Traces,Traces')
    [InlineData("", "Home,Inference,Traces,Insights")]
    [InlineData("Insights,Traces", "Home,Insights,Traces,Inference")]
    [InlineData("Home,Bogus,Traces,Traces", "Home,Traces,Inference,Insights")]
    public void TheTabOrderIsHomeThenThePersonsOrder(string stored, string want)
    {
        Assert.Equal(want, string.Join(",", MonitorNavigation.Ordered(stored).Select(MonitorNavigation.RawValue)));
    }

    [Fact]
    public void StoringAndMovingTabsMatchesTheMac()
    {
        // probe: stored(default) = Inference,Traces,Insights;
        // moving(insights->inference) = Home,Insights,Inference,Traces; Home never moves.
        Assert.Equal("Inference,Traces,Insights", MonitorNavigation.Stored(MonitorNavigation.Ordered("")));
        Assert.Equal(
            new[] { MonitorTab.Home, MonitorTab.Insights, MonitorTab.Inference, MonitorTab.Traces },
            MonitorNavigation.Moving(MonitorTab.Insights, MonitorTab.Inference, DefaultOrder));
        Assert.Equal(DefaultOrder, MonitorNavigation.Moving(MonitorTab.Home, MonitorTab.Traces, DefaultOrder));
    }

    [Theory]
    // TraceCommonsAppMain.swift:300-303: Cmd-N is the Nth tab in the strip,
    // Home always Cmd-1; Ctrl here. Ctrl+5 opens nothing (spec W2-D5).
    [InlineData("", 1, MonitorTab.Home)]
    [InlineData("", 2, MonitorTab.Inference)]
    [InlineData("", 3, MonitorTab.Traces)]
    [InlineData("", 4, MonitorTab.Insights)]
    [InlineData("Insights,Traces", 2, MonitorTab.Insights)]
    [InlineData("Insights,Traces", 4, MonitorTab.Inference)]
    public void CtrlDigitOpensTheTabAtThatPosition(string order, int digit, MonitorTab want)
    {
        Assert.Equal(want, MonitorNavigation.TabForDigit(digit, order));
        Assert.Equal(digit, MonitorNavigation.DigitFor(want, order));
    }

    [Fact]
    public void CtrlFiveIsGone()
    {
        Assert.Null(MonitorNavigation.TabForDigit(5, ""));
        Assert.Null(MonitorNavigation.TabForDigit(0, ""));
    }

    [Theory]
    // probe: inspectorAvailable(...) for each tab and Home page.
    [InlineData(MonitorTab.Home, HomePage.Overview, false)]
    [InlineData(MonitorTab.Home, HomePage.History, true)]
    [InlineData(MonitorTab.Home, HomePage.MissionDrafts, false)]
    [InlineData(MonitorTab.Inference, HomePage.Overview, false)]
    [InlineData(MonitorTab.Traces, HomePage.Overview, true)]
    [InlineData(MonitorTab.Insights, HomePage.Overview, false)]
    public void TheInspectorIsAvailableWhereTheMacHasIt(MonitorTab tab, HomePage page, bool want)
    {
        Assert.Equal(want, MonitorNavigation.InspectorAvailable(tab, page));
    }

    [Theory]
    // probe: graphAvailable / promptsInInspector.
    [InlineData(MonitorTab.Home, false, true)]
    [InlineData(MonitorTab.Inference, false, false)]
    [InlineData(MonitorTab.Traces, true, false)]
    [InlineData(MonitorTab.Insights, false, false)]
    public void TheGraphIsTracesOnlyAndThePromptsAreHomes(MonitorTab tab, bool graph, bool prompts)
    {
        Assert.Equal(graph, MonitorNavigation.GraphAvailable(tab));
        Assert.Equal(prompts, MonitorNavigation.PromptsInInspector(tab));
    }

    [Fact]
    public void OnboardingShowsInferenceAndHiddenMissionsFallBack()
    {
        // probe: shown(requiresOnboarding) = [Inference]; missionDrafts.shown(false) = overview.
        Assert.Equal(MonitorTab.Inference, MonitorNavigation.ShownTab(MonitorTab.Traces, requiresOnboarding: true));
        Assert.Equal(MonitorTab.Traces, MonitorNavigation.ShownTab(MonitorTab.Traces, requiresOnboarding: false));
        Assert.Equal(HomePage.Overview, HomePage.MissionDrafts.Shown(false));
        Assert.Equal(HomePage.MissionDrafts, HomePage.MissionDrafts.Shown(true));
    }

    [Fact]
    public void HistoryIsTracesPageBehindTheBreadcrumb()
    {
        // MonitorWindowView.swift:755-757 and :788-791: History lives on Traces,
        // with Traces to go back to.
        MonitorLocation history = MonitorNavigation.Land(MonitorDestination.History, MonitorLocation.Default, missions: true);
        Assert.Equal(new MonitorLocation(MonitorTab.Traces, HomePage.Overview, TracesPage.History), history);
        IReadOnlyList<Crumb>? crumbs = MonitorNavigation.Breadcrumb(history, missions: true);
        Assert.NotNull(crumbs);
        Assert.Equal(new[] { new Crumb(CrumbWord.Tab, MonitorTab.Traces, true), new Crumb(CrumbWord.History, MonitorTab.Traces, false) }, crumbs);
        Assert.Equal(TracesPage.Tree, MonitorNavigation.Back(history, crumbs![0]).TracesPage);
        Assert.Equal(NoticePlace.History, MonitorNavigation.Place(history));
    }

    [Fact]
    public void MissionDraftsIsHomesPageBehindTheBreadcrumb()
    {
        // MonitorWindowView.swift:766-767 with HomeViews.swift:69-74 (the crumb is
        // the core's missions word); hidden missions fall back to the overview.
        MonitorLocation drafts = MonitorNavigation.Land(MonitorDestination.MissionDrafts, MonitorLocation.Default, missions: true);
        Assert.Equal(HomePage.MissionDrafts, drafts.HomePage);
        Assert.Equal(
            new[] { new Crumb(CrumbWord.Tab, MonitorTab.Home, true), new Crumb(CrumbWord.Missions, MonitorTab.Home, false) },
            MonitorNavigation.Breadcrumb(drafts, missions: true));
        Assert.Null(MonitorNavigation.Breadcrumb(drafts, missions: false));
        Assert.Equal(HomePage.Overview, MonitorNavigation.Land(MonitorDestination.MissionDrafts, MonitorLocation.Default, missions: false).HomePage);
        foreach (MonitorTab tab in DefaultOrder)
        {
            Assert.Null(MonitorNavigation.Breadcrumb(MonitorLocation.Default with { Tab = tab }, missions: true));
        }
    }

    [Theory]
    // MonitorNotices.swift:127-136.
    [InlineData(MonitorTab.Home, HomePage.Overview, TracesPage.Tree, NoticePlace.Home)]
    [InlineData(MonitorTab.Home, HomePage.History, TracesPage.Tree, NoticePlace.HomeHistory)]
    [InlineData(MonitorTab.Inference, HomePage.Overview, TracesPage.Tree, NoticePlace.PrivateAI)]
    [InlineData(MonitorTab.Traces, HomePage.Overview, TracesPage.Tree, NoticePlace.Traces)]
    [InlineData(MonitorTab.Traces, HomePage.Overview, TracesPage.History, NoticePlace.History)]
    [InlineData(MonitorTab.Insights, HomePage.Overview, TracesPage.Tree, NoticePlace.Insights)]
    public void EachTabsNoticesGoWhereTheMacPutsThem(MonitorTab tab, HomePage home, TracesPage traces, NoticePlace want)
    {
        Assert.Equal(want, MonitorNavigation.Place(new MonitorLocation(tab, home, traces)));
    }

    // ---- Inspector demand ------------------------------------------------

    [Fact]
    public void DemandKeysAreTheMacs()
    {
        // probe: keys(approvalUndo: [e1,e2], contributed: e1, kept: e3,
        // contributedFolder: p1, submittingFolder: p2) and its opens(...) answers.
        IReadOnlySet<string> keys = InspectorDemand.Keys(new DemandSources(
            ApprovalUndo: new[] { "e1", "e2" }, Contributed: "e1", Kept: "e3", ContributedFolder: "p1", SubmittingFolder: "p2"));
        Assert.Equal(new[] { "contributed:e1", "folder-submit:p2", "folder-undo:p1", "kept:e3", "undo:e1,e2" }, keys.OrderBy(k => k, StringComparer.Ordinal));
        Assert.True(InspectorDemand.Opens(new HashSet<string>(), keys));
        Assert.False(InspectorDemand.Opens(keys, keys));
        Assert.False(InspectorDemand.Opens(keys, new HashSet<string>()));
    }

    [Fact]
    public void ASecondApprovalOfTheSameSessionIsANewDemand()
    {
        // TracesInspectorHost.swift:176-180; probe "undo-at-format".
        var at = DateTimeOffset.FromUnixTimeMilliseconds(1_731_900_000_500);
        IReadOnlySet<string> first = InspectorDemand.Keys(new DemandSources(ApprovalUndo: new[] { "e1" }, ApprovedAt: at));
        Assert.Contains("undo:approval-at:1731900000.5", first);
        IReadOnlySet<string> again = InspectorDemand.Keys(new DemandSources(ApprovalUndo: new[] { "e1" }, ApprovedAt: at.AddSeconds(30)));
        Assert.True(InspectorDemand.Opens(first, again));
        Assert.Contains("review:s1", InspectorDemand.Keys(new DemandSources(Review: "s1")));
    }

    [Fact]
    public void OffersAskForTheInspectorOnlyWhereItDrawsThem()
    {
        // TracesInspectorHost.swift:193-199, counted only where
        // promptsInInspector (MonitorWindowView.swift:652-654).
        Assert.Equal(new[] { "offer:arming:p1", "offer:private-ai" },
            InspectorDemand.OfferKeys(MonitorTab.Home, true, "p1").OrderBy(k => k, StringComparer.Ordinal));
        Assert.Empty(InspectorDemand.OfferKeys(MonitorTab.Traces, true, "p1"));
    }

    [Fact]
    public void TheTrackerOpensOnAppearAndOnlyForSomethingNew()
    {
        // TracesInspectorHost.swift:206-208 and MonitorShell.swift:62-65: a
        // demand going away never closes the inspector, so it is never asked to.
        var tracker = new InspectorDemandTracker();
        var undo = new HashSet<string> { "undo:e1" };
        Assert.True(tracker.Update(undo));
        Assert.False(tracker.Update(undo));
        Assert.False(tracker.Update(new HashSet<string>()));
        Assert.True(tracker.Update(undo));

        var quiet = new InspectorDemandTracker();
        Assert.False(quiet.Update(new HashSet<string>()));
    }

    // ---- Notice stack ----------------------------------------------------

    [Fact]
    public void TheStackDrawsThreeFrontFirstAndPeeksEight()
    {
        // PopoverNotice.swift:201-207, probe: visible([a..e]) = [e, d, c].
        Assert.Equal(8, NoticeStack.Peek);
        Assert.Equal(3, NoticeStack.Drawn);
        Assert.Equal(new[] { "e", "d", "c" }, NoticeStack.Visible(new[] { "a", "b", "c", "d", "e" }));
        Assert.Equal(new[] { "a" }, NoticeStack.Visible(new[] { "a" }));
        Assert.Equal(1, NoticeStack.Direction(StackAnchor.Top));
        Assert.Equal(-1, NoticeStack.Direction(StackAnchor.Bottom));
    }

    [Fact]
    public void PlatesNarrowFourPercentALevelAtNinetyThenSeventyPercent()
    {
        // PopoverNotice.swift:258-260: scaleEffect(x: 1 - 0.04 * level), offset
        // peek * level in the stack's direction, opacity 0.9 then 0.7;
        // :236 the stack keeps peek * behind below the front card.
        IReadOnlyList<NoticePlate> plates = NoticeStack.Plates(new[] { "a", "b", "c", "d" }, StackAnchor.Top);
        Assert.Equal(new[] { new NoticePlate(1, 0.96, 8, 0.9), new NoticePlate(2, 0.92, 16, 0.7) }, plates);
        Assert.Equal(16, NoticeStack.PlateRoom(new[] { "a", "b", "c" }));
        Assert.Empty(NoticeStack.Plates(new[] { "a" }, StackAnchor.Top));
        Assert.Equal(-8, NoticeStack.Plates(new[] { "a", "b" }, StackAnchor.Bottom)[0].OffsetY);
    }

    [Fact]
    public void TheArrivalSpringIsTheMacs()
    {
        // PopoverNotice.swift:238: .spring(response: 0.36, dampingFraction: 0.86).
        Assert.Equal(0.36, NoticeStack.SpringResponse);
        Assert.Equal(0.86, NoticeStack.SpringDamping);
        Assert.True(NoticeStack.Slides("new", new HashSet<string> { "old" }));
        Assert.False(NoticeStack.Slides("old", new HashSet<string> { "old" }));
    }

    [Fact]
    public void ClosedHealthComesBackWhenItsCauseRecurs()
    {
        // MonitorNotices.swift:97-99.
        var closed = new HashSet<string> { "health:a", "health:b", "arming-offer:p1" };
        IReadOnlySet<string> pruned = NoticeStack.PrunedClosed(closed, new HashSet<string> { "health:a" });
        Assert.Equal(new[] { "arming-offer:p1", "health:a" }, pruned.OrderBy(k => k, StringComparer.Ordinal));
    }

    [Fact]
    public void TracesNoticesStackOffersBehindWhatIsHappeningNowWithActionsInFront()
    {
        // MonitorNotices.swift:140-161: app items, then the place's own, then
        // the action notice in front.
        NoticeKind[] present =
        {
            NoticeCatalog.ActionNotice, NoticeCatalog.Undo, NoticeCatalog.ArmingOffer("p1"),
            NoticeCatalog.Update, NoticeCatalog.PrivateAIOffer, NoticeCatalog.GateHeld,
        };
        Assert.Equal(
            new[] { "update", "arming-offer:p1", "undo", "gate-held", "action-notice" },
            NoticeCatalog.Order(NoticePlace.Traces, present).Select(card => card.Id));
        Assert.Equal(
            new[] { "update", "private-ai-offer", "action-notice" },
            NoticeCatalog.Order(NoticePlace.PrivateAI, present).Select(card => card.Id));
        Assert.Equal(
            new[] { "update", "action-notice" },
            NoticeCatalog.Order(NoticePlace.Insights, present).Select(card => card.Id));
        Assert.Equal(
            new[] { "update", "undo", "gate-held" },
            NoticeCatalog.Shown(NoticePlace.Traces, present, new HashSet<string> { "arming-offer:p1", "action-notice" }).Select(card => card.Id));
    }

    [Fact]
    public void EachCardsXFollowsTheMacsRule()
    {
        // MonitorNotices.swift:26-34 and each card's own line.
        Assert.Equal(NoticeClose.None, NoticeCatalog.GateHeld.Close);
        Assert.Equal(NoticeClose.PutsAway, NoticeCatalog.Undo.Close);
        Assert.Equal(NoticeClose.PutsAway, NoticeCatalog.GrantVoid("v").Close);
        Assert.Equal(NoticeClose.PutsAway, NoticeCatalog.ArmingRewording("r").Close);
        Assert.Equal(NoticeClose.ForNow, NoticeCatalog.ArmingOffer("p").Close);
        Assert.Equal(NoticeClose.Declines, NoticeCatalog.PrivateAIOffer.Close);
        Assert.Equal(NoticeClose.PutsAway, NoticeCatalog.LegacyNotice.Close);
        Assert.Equal(NoticeClose.PutsAway, NoticeCatalog.ActionNotice.Close);
    }

    // ---- Motion ----------------------------------------------------------

    [Theory]
    // probe: UnitCurve.bezier(0.2, 0.8, 0.2, 1).value(at:), GlassMotion.swift:7-12.
    [InlineData(0.1, 0.396692276)]
    [InlineData(0.25, 0.767284393)]
    [InlineData(0.5, 0.946079254)]
    [InlineData(0.75, 0.991107941)]
    [InlineData(0.9, 0.998829842)]
    public void ThePaneCurveIsTheTokenCurve(double t, double want)
    {
        Assert.Equal(0.22, MonitorMotion.Standard);
        Assert.Equal(want, MonitorMotion.Ease(t), 5);
    }

    [Fact]
    public void APaneMoveIsInstantUnderReduceMotionOrWhenTheMainPaneIsSqueezed()
    {
        // ThreePane.swift:539-545.
        Assert.True(MonitorMotion.AnimatesPaneMove(animationsEnabled: true, frameChanges: true, mainChanges: false));
        Assert.False(MonitorMotion.AnimatesPaneMove(animationsEnabled: false, frameChanges: true, mainChanges: false));
        Assert.False(MonitorMotion.AnimatesPaneMove(animationsEnabled: true, frameChanges: false, mainChanges: false));
        Assert.False(MonitorMotion.AnimatesPaneMove(animationsEnabled: true, frameChanges: true, mainChanges: true));
    }

    // ---- Growth at the screen edge (physical pixels) ----------------------

    [Theory]
    // The Mac's widths (probe: 440 -> 743 opening the inspector, 743 -> 440
    // closing it) at 100%, 125% and 150%, applied to an outer frame 16 px wider
    // than its client area (Windows' resize borders).
    [InlineData(1.0, 440, 743)]
    [InlineData(1.25, 550, 929)]
    [InlineData(1.5, 660, 1115)]
    public void OpeningTheInspectorGrowsTheOuterFrameByThePanesPixels(double scale, int clientPx, int grownPx)
    {
        var outer = new LayoutRect(100, 40, clientPx + 16, 700);
        var work = new LayoutRect(0, 0, 3840, 2100);
        PaneMove open = MonitorWindowGrowth.Plan(outer, 440, work, scale, new PaneVisibility(false, false), new PaneVisibility(false, true), 440, 440, true);
        Assert.Equal(new LayoutRect(100, 40, grownPx + 16, 700), open.Frame);
        Assert.True(open.Animates);

        PaneMove close = MonitorWindowGrowth.Plan(open.Frame, 743, work, scale, new PaneVisibility(false, true), new PaneVisibility(false, false), 440, 440, true);
        Assert.Equal(outer, close.Frame);
    }

    [Fact]
    public void AtTheRightEdgeTheWindowSlidesLeftOnItsOwnMonitor()
    {
        // ThreePane.swift:194-202 on a second monitor whose work area starts at
        // x = 1920 px: opening the inspector at 150% moves the window left so
        // its right edge sits on the work area's.
        var work = new LayoutRect(1920, 0, 2560, 1400);
        var outer = new LayoutRect(1920 + 2560 - 676, 40, 676, 700);
        PaneMove open = MonitorWindowGrowth.Plan(outer, 440, work, 1.5, new PaneVisibility(false, false), new PaneVisibility(false, true), 440, 440, true);
        Assert.Equal(work.Right, open.Frame.Right);
        Assert.Equal(1131, open.Frame.Width);
    }

    [Fact]
    public void AWindowHeldToANarrowScreenChangesAtOnce()
    {
        // ThreePane.swift:536-545: held to the screen, the map squeezes the
        // main pane, so nothing animates. 900 px wide work area, 100%.
        var work = new LayoutRect(0, 0, 900, 800);
        var outer = new LayoutRect(0, 0, 456, 700);
        PaneMove open = MonitorWindowGrowth.Plan(outer, 440, work, 1.0, new PaneVisibility(false, false), new PaneVisibility(true, false), 440, 440, true);
        Assert.Equal(900, open.Frame.Width);
        Assert.True(open.Animates, "the main pane keeps 440 beside a 434 wide map");

        // A main pane 600 wide (the window was widened without the map) is
        // squeezed to 514 beside the map when the window is held to 900.
        var wide = new LayoutRect(0, 0, 616, 700);
        PaneMove squeezed = MonitorWindowGrowth.Plan(wide, 600, work, 1.0, new PaneVisibility(false, false), new PaneVisibility(true, false), 440, 440, true);
        Assert.Equal(900, squeezed.Frame.Width);
        Assert.False(squeezed.Animates);
        Assert.False(MonitorWindowGrowth.Plan(outer, 440, work, 1.0, new PaneVisibility(false, false), new PaneVisibility(false, true), 440, 440, false).Animates);
    }

    [Fact]
    public void ThePassthroughRegionsCoverEachControlInPhysicalPixels()
    {
        // InputNonClientPointerSource.SetRegionRects takes physical pixels; a
        // region rounds outward so it always covers its control.
        var rects = TitleBarRegions.Passthrough(new[] { new LayoutRect(10.5, 6, 80.2, 32), new LayoutRect(0, 0, 0, 10) }, 1.25);
        Assert.Equal(new[] { (13, 7, 101, 41) }, rects); // 10.5*1.25 = 13.125 down, 90.7*1.25 = 113.375 up; 6*1.25 = 7.5 down, 38*1.25 = 47.5 up
    }

    [Fact]
    public void CaptionInsetKeepsItsLastWidthWhenTheTitleBarReportsNoneWhileRestoring()
    {
        // Restoring a minimised window raises AppWindow.Changed with
        // RightInset -8 (test box, 2026-11-19). GridLength threw on it inside
        // that event, which fails the process fast; read as 0 instead, the
        // toolbar slid under the caption buttons and stayed there, because no
        // later size change re-read the inset, so a click on the inspector
        // toggle maximised the window. The last good width is kept.
        Assert.Equal(110.4, TitleBarRegions.InsetDips(138, 1.25, 0), 9);
        Assert.Equal(0, TitleBarRegions.InsetDips(0, 1.0, 144));
        Assert.Equal(144, TitleBarRegions.InsetDips(-8, 1.0, 144));
        Assert.Equal(110.4, TitleBarRegions.InsetDips(138, 0, 110.4), 9);
        Assert.Equal(110.4, TitleBarRegions.InsetDips(138, double.NaN, 110.4), 9);
        Assert.Equal(144, TitleBarRegions.InsetDips(double.PositiveInfinity, 1.0, 144));
    }

    [Fact]
    public void NoPassthroughRegionsAtAScaleThatIsNotAPositiveNumber()
    {
        var control = new[] { new LayoutRect(10, 6, 80, 32) };
        Assert.Empty(TitleBarRegions.Passthrough(control, 0));
        Assert.Empty(TitleBarRegions.Passthrough(control, double.NaN));
        Assert.Empty(TitleBarRegions.Passthrough(control, double.PositiveInfinity));
    }

    // ---- Clocks ----------------------------------------------------------

    [Fact]
    public void ClocksStopWhenHiddenToTheTrayOrMinimisedAndRestartOnlyWhenBothClear()
    {
        var gate = new WindowClockGate();
        var seen = new List<bool>();
        gate.Changed += seen.Add;
        Assert.True(gate.MayTick);
        gate.SetHiddenToTray(true);
        gate.SetMinimised(true);
        gate.SetHiddenToTray(false);
        Assert.False(gate.MayTick);
        gate.SetMinimised(false);
        Assert.True(gate.MayTick);
        gate.SetClosed();
        Assert.Equal(new[] { false, true, false }, seen);
    }
}
