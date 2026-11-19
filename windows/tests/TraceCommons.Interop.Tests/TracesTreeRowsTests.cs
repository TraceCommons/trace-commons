using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Traces tree's rows, keyboard, search, drag and row words, against
/// the Mac's <c>TracesTreeView</c> (TracesViews.swift) and its own tests
/// (TracesTreeTests.swift, TracesResponsivenessTests.swift,
/// TracesFolderOrderTests.swift), over the recorded sample sets and the
/// Mac's probed trees (mac-map-probe.json). Formatter output is pinned to
/// the throwaway Swift probe in this file's <see cref="MacFormatters"/>.
/// </summary>
public class TracesTreeRowsTests
{
    private static readonly CultureInfo EnUs = CultureInfo.GetCultureInfo("en-US");

    private static TreeWords Words => TreeWords.Core.Value;

    private static IReadOnlySet<string> Set(params string[] ids) => new HashSet<string>(ids, StringComparer.Ordinal);

    private static async Task<TracesTree> Tree(string set) =>
        (await FlowMapSceneTests.Read(set)).Tree(MapFeed.CoreScansWhenUnset);

    [Fact]
    public async Task TheRowsAreEveryFolderAndTheTracesOfTheOpenOnesAsTheMacDrawsThem()
    {
        // TracesTreeTests.swift:216-226 over the Mac's probed tree for the
        // same recorded set: folders in order, each open folder's traces after it.
        TracesTree tree = await Tree("busyQueue");
        JsonElement mac = MapProbe.Set("busyQueue").GetProperty("tree").GetProperty("folders");
        var expected = new List<string>();
        foreach (JsonElement folder in mac.EnumerateArray())
        {
            expected.Add("folder:" + folder.GetProperty("id").GetString());
            expected.AddRange(folder.GetProperty("sessions").EnumerateArray().Select(s => "session:" + s.GetString()));
        }

        IReadOnlySet<string> all = new HashSet<string>(tree.Folders.Select(f => f.Id));
        Assert.Equal(expected, TracesTreeRows.VisibleRows(tree, all).Select(r => r.RawValue));
        Assert.Equal(expected, TracesTreeRows.DrawnRows(tree, all).Select(r => r.Id));

        // Every folder starts closed (MonitorShellTests.swift:83-92): folder rows only.
        Assert.Equal(
            mac.EnumerateArray().Select(f => "folder:" + f.GetProperty("id").GetString()),
            TracesTreeRows.VisibleRows(tree, Set()).Select(r => r.RawValue));
    }

    [Fact]
    public async Task TheKeyboardWalksFoldersAndTracesAsDrawn()
    {
        // TracesTreeTests.swift:216-255.
        TracesTree tree = await Tree("normalDay");
        IReadOnlySet<string> all = new HashSet<string>(tree.Folders.Select(f => f.Id));
        IReadOnlyList<TreeSelection> rows = TracesTreeRows.VisibleRows(tree, all);
        TracesTree.FolderNode first = tree.Folders.First(f => f.Sessions.Count > 0);
        var folder = new TreeSelection.Folder(first.Id);
        var firstSession = new TreeSelection.Session(first.Sessions[0].EntryId);
        Assert.Equal(firstSession, TracesTreeRows.Moved(folder, true, rows));
        Assert.Equal(folder, TracesTreeRows.Moved(firstSession, false, rows));
        Assert.Equal(rows[0], TracesTreeRows.Moved(null, true, rows));
        Assert.Equal(rows[0], TracesTreeRows.Moved(new TreeSelection.Session("gone"), false, rows));
        Assert.Equal(rows[^1], TracesTreeRows.Moved(rows[^1], true, rows));
        Assert.Equal(rows[0], TracesTreeRows.Moved(rows[0], false, rows));
        Assert.Null(TracesTreeRows.Moved(null, true, Array.Empty<TreeSelection>()));

        // A closed folder keeps its row and hides its traces.
        var closedSet = new HashSet<string>(all);
        closedSet.Remove(first.Id);
        IReadOnlyList<TreeSelection> closed = TracesTreeRows.VisibleRows(tree, closedSet);
        Assert.Contains(folder, closed);
        Assert.DoesNotContain(firstSession, closed);
    }

    [Fact]
    public void ClickingTheSelectedRowAgainSelectsNothing()
    {
        // TracesTreeTests.swift:170-181.
        var folder = new TreeSelection.Folder("p1");
        var session = new TreeSelection.Session("e1");
        Assert.Equal(folder, TracesTreeRows.Toggled(folder, null));
        Assert.Null(TracesTreeRows.Toggled(folder, folder));
        Assert.Equal(folder, TracesTreeRows.Toggled(folder, session));
        Assert.Equal(session, TracesTreeRows.Toggled(session, null));
        Assert.Null(TracesTreeRows.Toggled(session, session));
        // A folder and a trace with the same id are different rows.
        Assert.Equal(new TreeSelection.Session("p1"), TracesTreeRows.Toggled(new TreeSelection.Session("p1"), folder));
    }

    [Fact]
    public void TheStoredSelectionIsTheMacs()
    {
        // TracesTree.swift:333-349.
        Assert.Equal("folder:p", new TreeSelection.Folder("p").RawValue);
        Assert.Equal("session:e", new TreeSelection.Session("e").RawValue);
        Assert.Equal(new TreeSelection.Folder("a:b"), TreeSelection.Parse("folder:a:b"));
        Assert.Null(TreeSelection.Parse("folder:"));
        Assert.Null(TreeSelection.Parse("e1"));
    }

    [Fact]
    public void ASearchOpensWhatItMatchedAndClearingClosesOnlyThat()
    {
        // TracesResponsivenessTests.swift:135-168.
        var opened = TracesTreeRows.SearchExpansion("min", new[] { "a", "b" }, Set("b", "c"), Set());
        Assert.Equal(new[] { "a", "b", "c" }, opened.Expanded.OrderBy(x => x));
        Assert.Equal(new[] { "a" }, opened.SearchOpened);

        var typed = TracesTreeRows.SearchExpansion("m", new[] { "a", "b", "d" }, Set("b", "c"), Set());
        var narrowed = TracesTreeRows.SearchExpansion("mi", new[] { "a" }, typed.Expanded, typed.SearchOpened);
        var cleared = TracesTreeRows.SearchExpansion("", Array.Empty<string>(), narrowed.Expanded, narrowed.SearchOpened);
        Assert.Equal(new[] { "b", "c" }, cleared.Expanded.OrderBy(x => x));
        Assert.Empty(cleared.SearchOpened);

        var typed2 = TracesTreeRows.SearchExpansion("m", new[] { "a", "b" }, Set(), Set());
        var kept = TracesTreeRows.SearchExpansion("", Array.Empty<string>(), typed2.Expanded, typed2.SearchOpened, keep: "a");
        Assert.Equal(new[] { "a" }, kept.Expanded);

        var blank = TracesTreeRows.SearchExpansion("   ", new[] { "a" }, Set("c"), Set());
        Assert.Equal(new[] { "c" }, blank.Expanded);
        Assert.Empty(blank.SearchOpened);
    }

    [Fact]
    public void ADraggedFolderTradesPlacesPastItsNeighboursMiddle()
    {
        // TracesFolderOrderTests.swift:50-57.
        (string, double) above = ("a", 20), below = ("c", 100);
        Assert.Null(TracesTreeRows.FolderSwap(60, above, below));
        Assert.Equal("a", TracesTreeRows.FolderSwap(19, above, below));
        Assert.Equal("c", TracesTreeRows.FolderSwap(101, above, below));
        Assert.Null(TracesTreeRows.FolderSwap(-50, null, below));
        Assert.Null(TracesTreeRows.FolderSwap(500, above, null));
        // At a neighbour's middle exactly, not yet past it.
        Assert.Null(TracesTreeRows.FolderSwap(20, above, below));
        Assert.Null(TracesTreeRows.FolderSwap(100, above, below));
    }

    [Fact]
    public void TheTreesMotionIsTheMacs()
    {
        // TracesViews.swift:503 (GlassMotion.fast), :717 (6 pt), :764
        // (spring 0.3/0.85); NudgeViews.swift:153, :161 (fast); GlassTokens
        // Motion.fast 0.15.
        Assert.Equal(6, TracesTreeRows.FolderDragThreshold);
        Assert.Equal(38, TracesTreeRows.NoteInset);
        Assert.Equal(new SpringCurve(0.3, 0.85), TracesTreeRows.FolderDragRelease);
        Assert.Equal(0.15, TracesTreeRows.ScrollToRow.Seconds);
        Assert.Equal(0.15, TracesTreeRows.SearchFieldSlide.Seconds);
        Assert.Equal(BezierCurve.Token(0.15), TracesTreeRows.ScrollToRow);
    }

    [Fact]
    public void AFolderSaysItsModeAndCountOrThatItIsIgnored()
    {
        // TracesViews.swift:944-953, with the core's words.
        static string Label(string mode) => mode == MapModes.Ask ? "Ask" : mode == MapModes.AutoUpload ? "Automatic" : "Never";
        var one = new TracesTree.FolderNode("p", "P", MapModes.Ask, new[] { Entry("e1") });
        Assert.Equal("Ask · " + Words.Count("sessions_waiting_one"), TracesTreeRows.FolderSub(one, Words, Label));
        Assert.Equal("Ask · 1 trace waiting", TracesTreeRows.FolderSub(one, Words, Label));
        var three = one with { Mode = MapModes.AutoUpload, Sessions = new[] { Entry("e1"), Entry("e2"), Entry("e3") } };
        Assert.Equal("Automatic · 3 traces waiting", TracesTreeRows.FolderSub(three, Words, Label));
        // A folder the core does not list has no mode word.
        Assert.Equal("1 trace waiting", TracesTreeRows.FolderSub(one with { Mode = null }, Words, Label));
        Assert.Equal(Words["ignored_folder"], TracesTreeRows.FolderSub(one with { Mode = MapModes.Ignore }, Words, Label));
        Assert.Equal("ignored · never queued", TracesTreeRows.FolderSub(one with { Mode = MapModes.Ignore }, Words, Label));
    }

    [Fact]
    public void ThePillAndItsLinesAreTheCoresWords()
    {
        // TracesViews.swift:956-964, :986-990.
        Assert.Equal("Submit · 4", TracesTreeRows.SubmitTitle(4, Words));
        Assert.Equal(Words["submit_tip"] + " Two are held.", TracesTreeRows.SubmitHelp("Two are held.", Words));
        Assert.Equal(Words["submit_tip"], TracesTreeRows.SubmitHelp(null, Words));
        Assert.Equal("3 eligible · Two are held.", TracesTreeRows.WithheldNote("Two are held.", 3, Words));
    }

    [Fact]
    public void ATracesTitleIsItsWeekdayAndTimeAsTheMacWritesIt()
    {
        // TracesViews.swift:976-981 through the probe's en_US, Los Angeles.
        // The Mac writes U+202F before the day period, as current ICU does;
        // an older ICU or NLS writes a space, so the gap is read as either.
        static string Gap(string s) => s.Replace('\u202F', ' ');
        TimeZoneInfo la = TimeZoneInfo.FindSystemTimeZoneById("America/Los_Angeles");
        foreach ((long seconds, string mac) in MacFormatters.WhenEnUsLosAngeles)
        {
            var at = DateTimeOffset.FromUnixTimeSeconds(seconds);
            Assert.Equal(Gap(mac), Gap(TracesTreeRows.When(Entry("e") with { DiscoveredAt = at }, la, EnUs)));
            // Found wins over started.
            Assert.Equal(Gap(mac), Gap(TracesTreeRows.When(Entry("e") with { DiscoveredAt = at, StartedAt = at.AddDays(-3) }, la, EnUs)));
            Assert.Equal(Gap(mac), Gap(TracesTreeRows.When(Entry("e") with { StartedAt = at }, la, EnUs)));
        }

        Assert.Equal("—", TracesTreeRows.When(Entry("e"), la, EnUs));
    }

    [Fact]
    public void ASizeIsTheMacsMemoryCount()
    {
        // ByteCountFormatter(.memory) through the probe, from 1 KB up. Below
        // it the Mac's formatter spells its own words; see Size's remarks.
        foreach ((long bytes, string mac) in MacFormatters.Sizes.Where(s => s.Bytes >= 1024))
        {
            Assert.Equal(mac, TracesTreeRows.Size(bytes, EnUs));
        }

        Assert.Equal("1,023 B", TracesTreeRows.Size(1023, EnUs));
    }

    [Fact]
    public void ATracesSubLineSaysItsSizeThenItsState()
    {
        // TracesViews.swift:1031-1056: held, then ineligible, then trimmed,
        // then the attestation sentence, else waiting.
        string held = Words.Word("held");
        Assert.Equal("Held", held);
        Assert.Equal("2 KB · Held", TracesTreeRows.SessionSub(2048, true, held, "Not this", true, "Clear", Words, EnUs));
        Assert.Equal("2 KB · Not this", TracesTreeRows.SessionSub(2048, false, held, "Not this", true, "Clear", Words, EnUs));
        Assert.Equal("2 KB · trimmed to fit", TracesTreeRows.SessionSub(2048, false, held, null, true, "Clear", Words, EnUs));
        Assert.Equal("2 KB · Clear", TracesTreeRows.SessionSub(2048, false, held, null, false, "Clear", Words, EnUs));
        Assert.Equal("2 KB · waiting", TracesTreeRows.SessionSub(2048, false, held, null, false, null, Words, EnUs));
        Assert.Equal("waiting", TracesTreeRows.SessionSub(null, false, held, null, false, "", Words, EnUs));
    }

    [Fact]
    public void ATracesFlagIsAmberWhenItWaitsOnAPersonAndGreenOnlyForAClearMark()
    {
        // TracesViews.swift:1008-1024.
        Assert.True(TracesTreeRows.IsHeld(true, false, false));
        Assert.True(TracesTreeRows.IsHeld(false, true, false));
        Assert.True(TracesTreeRows.IsHeld(false, false, true));
        Assert.False(TracesTreeRows.IsHeld(false, false, false));
        Assert.True(TracesTreeRows.IsHeld("second-look-review-required"));
        Assert.True(TracesTreeRows.IsHeld("scrub-check-manual"));
        Assert.True(TracesTreeRows.IsHeld("returned-from-keep"));
        Assert.False(TracesTreeRows.IsHeld("kept-on-this-mac"));
        Assert.False(TracesTreeRows.IsHeld("witness-risk-review-required"));
        Assert.False(TracesTreeRows.IsHeld(null));
        Assert.Equal(TreeFlag.Ask, TracesTreeRows.Flag(true, false, TreeAttestationTone.Clear));
        Assert.Equal(TreeFlag.Ask, TracesTreeRows.Flag(false, true, TreeAttestationTone.Clear));
        Assert.Equal(TreeFlag.Ask, TracesTreeRows.Flag(false, false, TreeAttestationTone.Attention));
        Assert.Equal(TreeFlag.Ask, TracesTreeRows.Flag(false, false, TreeAttestationTone.Refused));
        Assert.Equal(TreeFlag.On, TracesTreeRows.Flag(false, false, TreeAttestationTone.Clear));
        Assert.Null(TracesTreeRows.Flag(false, false, TreeAttestationTone.Plain));
        Assert.Null(TracesTreeRows.Flag(false, false, null));
    }

    // ---- The state the window drives ----------------------------------

    [Fact]
    public async Task SelectingATraceFromElsewhereOpensItsFolderAndScrollsToIt()
    {
        // TracesViews.swift:300-317, :479-497.
        TracesTree tree = await Tree("normalDay");
        TracesTree.FolderNode folder = tree.Folders.First(f => f.Sessions.Count > 0);
        var state = new TracesTreeState();
        state.SetTree(tree);
        Assert.Empty(state.Expanded);
        state.Select(new TreeSelection.Session(folder.Sessions[0].EntryId));
        Assert.Contains(folder.Id, state.Expanded);
        Assert.Equal("session:" + folder.Sessions[0].EntryId, state.ScrollTarget);
        Assert.Contains(state.Rows, r => r.Id == state.ScrollTarget);

        // The person closes it over the selection: it stays closed until the selection moves.
        state.ToggleOpen(folder.Id);
        state.SetTree(tree);
        Assert.DoesNotContain(folder.Id, state.Expanded);
    }

    [Fact]
    public async Task ATraceSelectedBeforeItIsReadIsRevealedWhenItArrives()
    {
        // TracesViews.swift:487-488 and :322-325.
        TracesTree tree = await Tree("normalDay");
        TracesTree.FolderNode folder = tree.Folders.First(f => f.Sessions.Count > 0);
        var state = new TracesTreeState();
        state.Select(new TreeSelection.Session(folder.Sessions[0].EntryId));
        Assert.Empty(state.Expanded);
        state.SetTree(tree);
        Assert.Contains(folder.Id, state.Expanded);
    }

    [Fact]
    public async Task TheArrowKeysMoveTheSelectionAndOpenOrCloseItsFolder()
    {
        // TracesViews.swift:522-543.
        TracesTree tree = await Tree("normalDay");
        TracesTree.FolderNode folder = tree.Folders.First(f => f.Sessions.Count > 0);
        var state = new TracesTreeState();
        state.SetTree(tree);
        state.Move(down: true);
        Assert.Equal(new TreeSelection.Folder(tree.Folders[0].Id), state.Selection);
        state.Select(new TreeSelection.Folder(folder.Id));
        state.Disclose(open: true);
        Assert.Contains(folder.Id, state.Expanded);
        state.Move(down: true);
        Assert.Equal(new TreeSelection.Session(folder.Sessions[0].EntryId), state.Selection);
        state.Disclose(open: false);
        Assert.DoesNotContain(folder.Id, state.Expanded);
        Assert.Equal(new TreeSelection.Session(folder.Sessions[0].EntryId), state.Selection);
    }

    [Fact]
    public async Task ASearchNarrowsTheRowsAndOpensWhatItMatched()
    {
        // TracesStore.swift:70 and TracesViews.swift:328-335.
        TracesTree tree = await Tree("busyQueue");
        var state = new TracesTreeState();
        state.SetTree(tree);
        state.SetSearch("a");
        Assert.Equal(tree.Matching("a").Folders.Select(f => f.Id), state.Shown.Folders.Select(f => f.Id));
        Assert.All(state.Shown.Folders, f => Assert.Contains(f.Id, state.Expanded));
        state.SetSearch(string.Empty);
        Assert.Empty(state.Expanded);
        Assert.Equal(tree.Folders.Count, state.Shown.Folders.Count);
    }

    [Fact]
    public async Task AFolderOpenedByHandDuringASearchStaysOpenAfterIt()
    {
        // TracesViews.swift:585-588 and :113-114: opening or closing a folder
        // by hand takes it out of what the search opened.
        TracesTree tree = await Tree("busyQueue");
        var state = new TracesTreeState();
        state.SetTree(tree);
        state.SetSearch("a");
        string folder = state.Shown.Folders[0].Id;
        state.ToggleOpen(folder);
        state.ToggleOpen(folder);
        state.SetSearch(string.Empty);
        Assert.Contains(folder, state.Expanded);
    }

    [Fact]
    public async Task AFolderDroppedOnAnotherTakesItsPlaceAndTheOrderIsThePersons()
    {
        // TracesStore.swift:93-98; TracesFolderOrderTests.swift:24-34.
        TracesTree tree = await Tree("busyQueue");
        Assert.True(tree.Folders.Count >= 3);
        string a = tree.Folders[0].Id, c = tree.Folders[2].Id;
        var state = new TracesTreeState();
        state.SetTree(tree);
        Assert.False(state.Arranged);
        Assert.True(state.MoveFolder(a, c));
        Assert.True(state.Arranged);
        Assert.Equal(new[] { tree.Folders[1].Id, c, a }, state.Shown.Folders.Take(3).Select(f => f.Id));
        Assert.False(state.MoveFolder(a, a));
        Assert.False(state.MoveFolder("gone", a));

        // Read back: the same order from the saved list.
        var again = new TracesTreeState(state.FolderOrder, arranged: true);
        again.SetTree(tree);
        Assert.Equal(state.Shown.Folders.Select(f => f.Id), again.Shown.Folders.Select(f => f.Id));
    }

    [Fact]
    public async Task TheOrderMenuKeepsTheCoresOrderOrThePersonsAndHidesIgnoredFolders()
    {
        // TracesStore.swift:32-36, :113-120; TracesTree.swift:156-161, :196-203.
        MapSnapshot snapshot = await FlowMapSceneTests.Read("busyQueue");
        var state = new TracesTreeState();
        state.Read(snapshot, MapFeed.CoreScansWhenUnset);
        Assert.True(state.ShowsIgnored);
        Assert.Equal(
            TracesTree.Build(snapshot.Entries!, snapshot.Projects!, snapshot.SourceModes, MapFeed.CoreScansWhenUnset, showsIgnored: true).Folders.Select(f => f.Id),
            state.Shown.Folders.Select(f => f.Id));

        state.ChooseQueueOrder();
        JsonElement mac = MapProbe.Set("busyQueue");
        Assert.Equal(mac.GetProperty("treeKeepsOrder").EnumerateArray().Select(e => e.GetString()),
            TracesTree.Build(snapshot.Entries!, snapshot.Projects!, snapshot.SourceModes, MapFeed.CoreScansWhenUnset, showsIgnored: false, keepsOrder: true).Folders.Select(f => f.Id));
        Assert.Equal(
            TracesTree.Build(snapshot.Entries!, snapshot.Projects!, snapshot.SourceModes, MapFeed.CoreScansWhenUnset, showsIgnored: true, keepsOrder: true).Folders.Select(f => f.Id),
            state.Shown.Folders.Select(f => f.Id));

        state.ShowIgnored(false);
        Assert.Equal(mac.GetProperty("treeKeepsOrder").EnumerateArray().Select(e => e.GetString()), state.Shown.Folders.Select(f => f.Id));

        // Your order, with nothing dragged yet: the folders as drawn.
        var drawn = state.Shown.Folders.Select(f => f.Id).ToList();
        state.ChooseArranged();
        Assert.True(state.Arranged);
        Assert.Equal(drawn, state.FolderOrder);
        Assert.Equal(drawn, state.Shown.Folders.Select(f => f.Id));

        // The core's words for the control.
        Assert.Equal("Order of waiting traces", TreeOrderWords.Label);
        Assert.Equal("Oldest first", TreeOrderWords.Queue);
        Assert.Equal("Your order", TreeOrderWords.Arranged);
    }

    [Fact]
    public async Task TheSelectionsToolIsATracesOwnOrItsFoldersOnTheMap()
    {
        // MonitorWindowView.swift:715-724.
        TracesTree tree = await Tree("busyQueue");
        TracesTree.FolderNode folder = tree.Folders.First(f => f.Sessions.Count > 0);
        MapEntry entry = folder.Sessions[0];
        string expected = entry.DeclaredSource is { } d && MapTools.Known(d) ? d : entry.Source;
        Assert.Equal(expected, TracesTreeRows.ToolOf(new TreeSelection.Session(entry.EntryId), tree));
        string? onMap = tree.Tools.FirstOrDefault(t => t.Folders.Any(x => x.Id == folder.Id))?.Id;
        Assert.NotNull(onMap);
        Assert.Equal(onMap, TracesTreeRows.ToolOf(new TreeSelection.Folder(folder.Id), tree));
        Assert.Null(TracesTreeRows.ToolOf(null, tree));
        Assert.Null(TracesTreeRows.ToolOf(new TreeSelection.Session("gone"), tree));
        // A declared tool wins over the adapter that read it.
        var t = new TracesTree(Array.Empty<TracesTree.ToolNode>(), Array.Empty<TracesTree.FolderNode>(),
            new[] { new TracesTree.FolderNode("p", "P", MapModes.Ask, new[] { new MapEntry("e", MapTools.ClaudeCode, MapTools.Codex, "p", "P", null, null) }) });
        Assert.Equal(MapTools.Codex, TracesTreeRows.ToolOf(new TreeSelection.Session("e"), t));
        Assert.Equal(MapTools.Codex, TracesTreeRows.ToolOf(new TreeSelection.Folder("p"), t));
    }

    [Fact]
    public void OneRowMenuIsOpenAtATime()
    {
        // TracesViews.swift:107-122.
        var state = new TracesTreeState();
        state.ToggleMenu(new TreeRowMenu.FolderMenu("p"));
        Assert.Equal(new TreeRowMenu.FolderMenu("p"), state.Menu);
        state.ToggleMenu(new TreeRowMenu.SessionMenu("e"));
        Assert.Equal(new TreeRowMenu.SessionMenu("e"), state.Menu);
        state.ToggleMenu(new TreeRowMenu.SessionMenu("e"));
        Assert.Null(state.Menu);
    }

    private static MapEntry Entry(string id) => new(id, "codex", null, "p", "P", null, null);
}

/// <summary>
/// The Mac's own formatter output, printed by a throwaway probe
/// (scratchpad w3-r2/tree-probe/probe.swift, macOS 27.2.1, en_US): the calls
/// TracesViews.swift:978-980 and :1060-1062 make.
/// </summary>
internal static class MacFormatters
{
    public static readonly (long Seconds, string Mac)[] WhenEnUsLosAngeles =
    {
        (1_763_593_440, "Wed 3:04\u202FPM"),
        (1_763_550_000, "Wed 3:00\u202FAM"),
        (1_763_600_400, "Wed 5:00\u202FPM"),
        (1_763_640_000, "Thu 4:00\u202FAM"),
    };

    public static readonly (long Bytes, string Mac)[] Sizes =
    {
        (0, "Zero KB"), (1, "1 byte"), (999, "999 bytes"), (1000, "1,000 bytes"), (1023, "1,023 bytes"),
        (1024, "1 KB"), (1536, "2 KB"), (10_000, "10 KB"), (102_400, "100 KB"), (1_000_000, "977 KB"),
        (1_048_576, "1 MB"), (1_572_864, "1.5 MB"), (12_345_678, "11.8 MB"), (1_073_741_824, "1 GB"),
        (5_000_000_000, "4.66 GB"),
    };
}
