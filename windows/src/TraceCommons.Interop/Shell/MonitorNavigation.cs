using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The Monitor window's tabs, in the Mac's default order
/// (<c>MonitorWindowView.Tab</c>, MonitorWindowView.swift:24-32). The
/// declaration order is the default order.
/// </summary>
public enum MonitorTab
{
    Home,
    Inference,
    Traces,
    Insights,
}

/// <summary>Home's pages (<c>HomePage</c>, MonitorNavigation.swift:6-21).</summary>
/// <remarks>
/// The Mac's Missions catalogue page is not here: this shell's only mission
/// screen is the local drafts inbox, as on Linux.
/// </remarks>
public enum HomePage
{
    Overview,
    /// <summary>
    /// Kept for a window restored to it. Nothing links here: History opens on
    /// Traces (MonitorNavigation.swift:23-29, owner 2026-10-12).
    /// </summary>
    History,
    MissionDrafts,
}

/// <summary>The Traces tab's pages (<c>TracesPage</c>, MonitorNavigation.swift:26-29).</summary>
public enum TracesPage
{
    Tree,
    History,
}

/// <summary>Where a tab's notices go (<c>MonitorNotices.Place</c>, MonitorNotices.swift:39-46).</summary>
public enum NoticePlace
{
    Home,
    PrivateAI,
    Traces,
    History,
    Insights,
    HomeHistory,
}

/// <summary>What is on screen in the main pane.</summary>
public readonly record struct MonitorLocation(MonitorTab Tab, HomePage HomePage, TracesPage TracesPage)
{
    public static MonitorLocation Default => new(MonitorTab.Home, HomePage.Overview, TracesPage.Tree);
}

/// <summary>Which word a breadcrumb crumb shows.</summary>
public enum CrumbWord
{
    /// <summary>A tab's title; the crumb goes back to that tab's first page.</summary>
    Tab,
    /// <summary>The core's <c>history</c> word.</summary>
    History,
    /// <summary>The core's <c>missions</c> word (<c>HomeTabView.hostedHeading</c>, HomeViews.swift:69-74).</summary>
    Missions,
}

/// <summary>
/// One crumb: its word, the tab whose title it shows when
/// <see cref="Word"/> is <see cref="CrumbWord.Tab"/>, and whether pressing it
/// goes back (the last crumb is the page itself and does nothing).
/// </summary>
public readonly record struct Crumb(CrumbWord Word, MonitorTab Tab, bool GoesBack);

/// <summary>
/// The Monitor window's pure navigation rules, ported from
/// <c>MonitorWindowView</c> and <c>MonitorNotices</c> (macos/Sources/TraceCommonsApp)
/// so each is tested without a window.
/// </summary>
public static class MonitorNavigation
{
    /// <summary>Every tab, in the default order.</summary>
    public static IReadOnlyList<MonitorTab> All { get; } =
        new[] { MonitorTab.Home, MonitorTab.Inference, MonitorTab.Traces, MonitorTab.Insights };

    /// <summary>
    /// The tab's identity, the Mac's raw value (MonitorWindowView.swift:25-32),
    /// used for the stored order.
    /// </summary>
    public static string RawValue(MonitorTab tab) => tab switch
    {
        MonitorTab.Home => "Home",
        MonitorTab.Inference => "Inference",
        MonitorTab.Traces => "Traces",
        MonitorTab.Insights => "Insights",
        _ => throw new ArgumentOutOfRangeException(nameof(tab)),
    };

    private static MonitorTab? FromRawValue(string raw) => raw switch
    {
        "Home" => MonitorTab.Home,
        "Inference" => MonitorTab.Inference,
        "Traces" => MonitorTab.Traces,
        "Insights" => MonitorTab.Insights,
        _ => null,
    };

    /// <summary>Every tab but Home may be dragged (MonitorWindowView.swift:60).</summary>
    public static bool Movable(MonitorTab tab) => tab != MonitorTab.Home;

    /// <summary>
    /// The tabs in the person's order: Home, the movable tabs as stored, then
    /// any the stored order does not name, in their default order. An unknown
    /// or repeated name is skipped (MonitorWindowView.swift:65-74).
    /// </summary>
    public static IReadOnlyList<MonitorTab> Ordered(string? stored)
    {
        var movable = new List<MonitorTab>();
        foreach (string name in (stored ?? string.Empty).Split(',', StringSplitOptions.RemoveEmptyEntries))
        {
            if (FromRawValue(name) is MonitorTab tab && Movable(tab) && !movable.Contains(tab))
            {
                movable.Add(tab);
            }
        }

        movable.AddRange(All.Where(tab => Movable(tab) && !movable.Contains(tab)));
        return new[] { MonitorTab.Home }.Concat(movable).ToArray();
    }

    /// <summary>The stored form of an order (MonitorWindowView.swift:77-79).</summary>
    public static string Stored(IEnumerable<MonitorTab> order) =>
        string.Join(",", order.Where(Movable).Select(RawValue));

    /// <summary>
    /// The order after dragging <paramref name="tab"/> onto
    /// <paramref name="target"/>'s place; Home never moves
    /// (MonitorWindowView.swift:83-91).
    /// </summary>
    public static IReadOnlyList<MonitorTab> Moving(MonitorTab tab, MonitorTab target, IReadOnlyList<MonitorTab> order)
    {
        int from = IndexOf(order, tab);
        int to = IndexOf(order, target);
        if (!Movable(tab) || !Movable(target) || tab == target || from < 0 || to < 0)
        {
            return order;
        }

        var moved = order.ToList();
        moved.RemoveAt(from);
        moved.Insert(to, tab);
        return moved;
    }

    private static int IndexOf(IReadOnlyList<MonitorTab> order, MonitorTab tab)
    {
        for (int i = 0; i < order.Count; i++)
        {
            if (order[i] == tab)
            {
                return i;
            }
        }

        return -1;
    }

    /// <summary>
    /// The tab a Ctrl+digit opens: the tab at that position in the strip,
    /// Home always first (<c>shortcut(_:order:)</c>,
    /// TraceCommonsAppMain.swift:300-303). Null for a digit past the last tab.
    /// </summary>
    public static MonitorTab? TabForDigit(int digit, string? order)
    {
        IReadOnlyList<MonitorTab> tabs = Ordered(order);
        return digit >= 1 && digit <= tabs.Count ? tabs[digit - 1] : null;
    }

    /// <summary>The digit a tab's accelerator carries (TraceCommonsAppMain.swift:300-303).</summary>
    public static int DigitFor(MonitorTab tab, string? order) => IndexOf(Ordered(order), tab) + 1;

    /// <summary>
    /// The tab drawn for the chosen one: Inference while onboarding is
    /// required (MonitorWindowView.swift:95-97).
    /// </summary>
    public static MonitorTab ShownTab(MonitorTab tab, bool requiresOnboarding) =>
        requiresOnboarding ? MonitorTab.Inference : tab;

    /// <summary>A mission page falls back to the overview while missions are hidden (MonitorNavigation.swift:17-20).</summary>
    public static HomePage Shown(this HomePage page, bool missions) =>
        page == HomePage.MissionDrafts && !missions ? HomePage.Overview : page;

    /// <summary>
    /// Whether the inspector may open on what is shown
    /// (MonitorWindowView.swift:668-675).
    /// </summary>
    public static bool InspectorAvailable(MonitorTab shown, HomePage homePage, bool insightsDetail = false) => shown switch
    {
        MonitorTab.Traces => true,
        MonitorTab.Home => homePage == HomePage.History,
        MonitorTab.Insights => insightsDetail,
        _ => false,
    };

    /// <summary>Whether the Graph toggle works: Traces only (MonitorWindowView.swift:679).</summary>
    public static bool GraphAvailable(MonitorTab tab) => tab == MonitorTab.Traces;

    /// <summary>
    /// Whether the inspector draws the Traces prompts: on Home
    /// (MonitorWindowView.swift:652-654). The offers' demand keys count only there.
    /// </summary>
    public static bool PromptsInInspector(MonitorTab tab) => tab == MonitorTab.Home;

    /// <summary>Whether History is the page on screen (MonitorWindowView.swift:658-660).</summary>
    public static bool ShowsHistory(MonitorLocation at) =>
        (at.Tab == MonitorTab.Traces && at.TracesPage == TracesPage.History)
        || (at.Tab == MonitorTab.Home && at.HomePage == HomePage.History);

    /// <summary>Where a tab's notices go (MonitorNotices.swift:127-136).</summary>
    public static NoticePlace Place(MonitorLocation at) => at.Tab switch
    {
        MonitorTab.Home => at.HomePage == HomePage.History ? NoticePlace.HomeHistory : NoticePlace.Home,
        MonitorTab.Inference => NoticePlace.PrivateAI,
        MonitorTab.Traces => at.TracesPage == TracesPage.History ? NoticePlace.History : NoticePlace.Traces,
        _ => NoticePlace.Insights,
    };

    /// <summary>
    /// The breadcrumb above a sub-page, or null on a tab's first page
    /// (MonitorWindowView.swift:749-769). Home's Missions catalogue has no
    /// page here.
    /// </summary>
    public static IReadOnlyList<Crumb>? Breadcrumb(MonitorLocation at, bool missions)
    {
        if (at.Tab == MonitorTab.Traces && at.TracesPage == TracesPage.History)
        {
            return new[] { new Crumb(CrumbWord.Tab, MonitorTab.Traces, true), new Crumb(CrumbWord.History, MonitorTab.Traces, false) };
        }

        if (at.Tab != MonitorTab.Home)
        {
            return null;
        }

        return at.HomePage.Shown(missions) switch
        {
            HomePage.History => new[] { new Crumb(CrumbWord.Tab, MonitorTab.Home, true), new Crumb(CrumbWord.History, MonitorTab.Home, false) },
            HomePage.MissionDrafts => new[] { new Crumb(CrumbWord.Tab, MonitorTab.Home, true), new Crumb(CrumbWord.Missions, MonitorTab.Home, false) },
            _ => null,
        };
    }

    /// <summary>Where a crumb that goes back lands: the tab's first page.</summary>
    public static MonitorLocation Back(MonitorLocation at, Crumb crumb) => crumb.Tab switch
    {
        MonitorTab.Home => at with { Tab = MonitorTab.Home, HomePage = HomePage.Overview },
        MonitorTab.Traces => at with { Tab = MonitorTab.Traces, TracesPage = TracesPage.Tree },
        _ => at with { Tab = crumb.Tab },
    };

    /// <summary>
    /// Where a destination the window is asked for lands
    /// (<c>MonitorWindowView.land</c>, MonitorWindowView.swift:782-): History
    /// lives on Traces. Each name is one the tray, a notification or a link
    /// already opens.
    /// </summary>
    public static MonitorLocation Land(MonitorDestination destination, MonitorLocation current, bool missions) => destination switch
    {
        MonitorDestination.Home => current with { Tab = MonitorTab.Home, HomePage = HomePage.Overview },
        MonitorDestination.MissionDrafts => current with { Tab = MonitorTab.Home, HomePage = HomePage.MissionDrafts.Shown(missions) },
        MonitorDestination.Inference => current with { Tab = MonitorTab.Inference },
        MonitorDestination.Traces => current with { Tab = MonitorTab.Traces, TracesPage = TracesPage.Tree },
        MonitorDestination.History => current with { Tab = MonitorTab.Traces, TracesPage = TracesPage.History },
        MonitorDestination.Insights => current with { Tab = MonitorTab.Insights },
        _ => current,
    };
}

/// <summary>Where an outside caller wants the window to go (<c>MonitorDestination</c>, MonitorNavigation.swift:32-48).</summary>
public enum MonitorDestination
{
    Home,
    MissionDrafts,
    Inference,
    Traces,
    History,
    Insights,
}
