using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A folder's Automatic switch updates only what changed (owner, 2026-11-20,
/// from the profiling in #2278): each press re-read the queue and rebuilt
/// every tree row and the map. The tree now builds again only the rows whose
/// drawn values changed, the queue's own refreshes draw once per burst, and
/// the map rebuilds its scene only when a read differs from the last one.
/// The daemon's reply stays authoritative: a confirmed press still reads
/// everything again, and a refused or unchanged write draws the row back as
/// the core has it.
/// </summary>
public class TreeRowKeysTests
{
    /// <summary>Every row's key as the tree view computes it, with the given folder writing or refused.</summary>
    private static List<(string Id, string Key)> Keys(
        TracesTreeState state, string? writing = null, string? refused = null,
        Func<string, TreeRowKeys.GroupFacts?>? group = null, Func<string, TreeRowKeys.LiveFacts?>? live = null) =>
        state.Rows.Select(row => row switch
        {
            TreeRow.FolderRow f => (row.Id, TreeRowKeys.Folder(
                f.Folder, state.Selection == new TreeSelection.Folder(f.Folder.Id), state.Expanded.Contains(f.Folder.Id),
                group?.Invoke(f.Folder.Id), f.Folder.Id == writing, f.Folder.Id == refused ? "refused" : null, null)),
            TreeRow.SessionRow s => (row.Id, TreeRowKeys.Session(
                s.Entry, s.Off, state.Selection == new TreeSelection.Session(s.Entry.EntryId), live?.Invoke(s.Entry.EntryId))),
            _ => throw new InvalidOperationException(),
        }).ToList();

    private static TreeRowPlan Plan(List<(string Id, string Key)> drawn, List<(string Id, string Key)> now, string? force = null) =>
        TreeRowKeys.Plan(drawn.Select(r => r.Id).ToList(), drawn.ToDictionary(r => r.Id, r => r.Key), now, force);

    private static async Task<TracesTreeState> Opened(string set)
    {
        var state = new TracesTreeState();
        state.SetTree((await FlowMapSceneTests.Read(set)).Tree(MapFeed.CoreScansWhenUnset));
        foreach (TracesTree.FolderNode folder in state.Tree.Folders.Where(f => !state.Expanded.Contains(f.Id)))
        {
            state.ToggleOpen(folder.Id);
        }

        return state;
    }

    private static TracesTree WithMode(TracesTree tree, string folderId, string mode) =>
        tree with { Folders = tree.Folders.Select(f => f.Id == folderId ? f with { Mode = mode } : f).ToList() };

    [Fact]
    public async Task APressRebuildsOneRow()
    {
        TracesTreeState state = await Opened("normalDay");
        Assert.True(state.Tree.Folders.Count >= 2);
        Assert.Contains(state.Rows, r => r is TreeRow.SessionRow);
        string folder = state.Tree.Folders[1].Id;
        string row = new TreeSelection.Folder(folder).RawValue;
        List<(string Id, string Key)> drawn = Keys(state);

        // The write starts: its switch turns off.
        List<(string Id, string Key)> writing = Keys(state, writing: folder);
        TreeRowPlan started = Plan(drawn, writing);
        Assert.Equal(new[] { row }, started.Build);
        Assert.True(started.InPlace);

        // Refused: the row is drawn back, with the refusal under it.
        Assert.Equal(new[] { row }, Plan(writing, Keys(state, refused: folder)).Build);

        // Confirmed: the map's read has the folder armed; only its row changes.
        string other = state.Tree.Folders[1].Mode == MapModes.AutoUpload ? MapModes.Ask : MapModes.AutoUpload;
        state.SetTree(WithMode(state.Tree, folder, other));
        TreeRowPlan armed = Plan(writing, Keys(state));
        Assert.Equal(new[] { row }, armed.Build);
        Assert.True(armed.InPlace);
    }

    [Fact]
    public async Task ASecondReadOfTheSameQueueRebuildsNothing()
    {
        // Every read makes new objects; equal values are the same row.
        TracesTreeState first = await Opened("normalDay");
        TracesTreeState second = await Opened("normalDay");
        TreeRowPlan plan = Plan(Keys(first), Keys(second));
        Assert.Empty(plan.Build);
        Assert.True(plan.InPlace);
    }

    [Fact]
    public async Task IgnoringAFolderRebuildsItsTracesToo()
    {
        TracesTreeState state = await Opened("normalDay");
        TracesTree.FolderNode folder = state.Tree.Folders.First(f => f.Sessions.Count > 0);
        List<(string Id, string Key)> drawn = Keys(state);
        state.SetTree(WithMode(state.Tree, folder.Id, MapModes.Ignore));
        List<(string Id, string Key)> now = Keys(state);
        TreeRowPlan plan = Plan(drawn, now);
        var expected = new HashSet<string>(now.Select(r => r.Id).Where(id =>
            id == new TreeSelection.Folder(folder.Id).RawValue
            || folder.Sessions.Any(s => id == new TreeSelection.Session(s.EntryId).RawValue)));
        Assert.Equal(expected, plan.Build.ToHashSet());
    }

    [Fact]
    public async Task TheQueuesOwnValuesAreKeyedToo()
    {
        TracesTreeState state = await Opened("normalDay");
        TracesTree.FolderNode folder = state.Tree.Folders.First(f => f.Sessions.Count > 0);
        string entry = folder.Sessions[0].EntryId;
        var offer = new TreeRowKeys.GroupFacts(true, 3, null);
        var live = new TreeRowKeys.LiveFacts(1024, null, true, null, false, null, false, false, true);
        List<(string Id, string Key)> drawn = Keys(state, group: _ => offer, live: _ => live);

        // The folder's offer moved: its row only.
        TreeRowPlan offered = Plan(drawn, Keys(state, group: id => id == folder.Id ? offer with { OfferedCount = 2 } : offer, live: _ => live));
        Assert.Equal(new[] { new TreeSelection.Folder(folder.Id).RawValue }, offered.Build);

        // One trace may no longer be sent: its row only.
        TreeRowPlan held = Plan(drawn, Keys(state, group: _ => offer, live: id => id == entry ? live with { MaySubmit = false } : live));
        Assert.Equal(new[] { new TreeSelection.Session(entry).RawValue }, held.Build);

        // Starting or ending a submit redraws just its trace, even if eligibility is unchanged.
        List<(string Id, string Key)> submitting = Keys(state, group: _ => offer, live: id => id == entry ? live with { IsSubmitting = true } : live);
        Assert.Equal(new[] { new TreeSelection.Session(entry).RawValue }, Plan(drawn, submitting).Build);
        Assert.Equal(new[] { new TreeSelection.Session(entry).RawValue }, Plan(submitting, drawn).Build);

        // The queue cleared under a refresh: every trace row has no live row.
        TreeRowPlan cleared = Plan(drawn, Keys(state, group: _ => offer));
        Assert.Equal(drawn.Count(r => r.Id.StartsWith(new TreeSelection.Session("").RawValue, StringComparison.Ordinal)), cleared.Build.Count);
    }

    [Fact]
    public async Task AForcedRowIsBuiltEvenUnchanged()
    {
        // The bucket's switch never writes; the row is drawn again so the switch goes back.
        TracesTreeState state = await Opened("normalDay");
        List<(string Id, string Key)> drawn = Keys(state);
        string row = drawn[0].Id;
        TreeRowPlan plan = Plan(drawn, drawn, force: row);
        Assert.Equal(new[] { row }, plan.Build);
        Assert.True(plan.InPlace);
        Assert.Empty(Plan(drawn, drawn, force: "folder:not-drawn").Build);
    }

    [Fact]
    public async Task OpeningAFolderBuildsOnlyItsNewRows()
    {
        TracesTreeState state = await Opened("normalDay");
        TracesTree.FolderNode folder = state.Tree.Folders.First(f => f.Sessions.Count > 0);
        state.ToggleOpen(folder.Id);
        List<(string Id, string Key)> closed = Keys(state);
        state.ToggleOpen(folder.Id);
        List<(string Id, string Key)> open = Keys(state);
        TreeRowPlan plan = Plan(closed, open);
        Assert.False(plan.InPlace);

        // The folder's own row (its chevron turns) and its traces.
        var expected = new HashSet<string>(folder.Sessions.Select(s => new TreeSelection.Session(s.EntryId).RawValue))
        {
            new TreeSelection.Folder(folder.Id).RawValue,
        };
        Assert.Equal(expected, plan.Build.ToHashSet());
    }

    [Fact]
    public async Task TheSameReadIsKnown()
    {
        MapSnapshot a = await FlowMapSceneTests.Read("normalDay");
        MapSnapshot b = await FlowMapSceneTests.Read("normalDay");
        Assert.True(MapSnapshot.SameRead(a, b));
        Assert.False(MapSnapshot.SameRead(a, await FlowMapSceneTests.Read("busyQueue")));
        Assert.False(MapSnapshot.SameRead(null, b));

        // A snapshot made without the replies is never taken for the same read.
        MapSnapshot bare = a with { Replies = null };
        Assert.False(MapSnapshot.SameRead(bare, bare));
    }

    [Fact]
    public void ThePressIsWiredToTheRowsAndTheMap()
    {
        string tree = ShellSource("TraceCommons.App/MainWindow.Tree.cs");
        string automatic = Between(tree, "private FolderAutomatic NewFolderAutomatic()", "\n    }\n");

        // A confirmed press still reads everything: the core's answer is the authority.
        Assert.Contains("await ViewModel.RefreshAsync();", automatic, StringComparison.Ordinal);
        Assert.Contains("await _map.ReadAsync();", automatic, StringComparison.Ordinal);
        Assert.Contains("writes.Changed += () => _tree?.Refresh();", automatic, StringComparison.Ordinal);

        // A press that wrote nothing draws its row back as it was.
        string press = Between(tree, "async void ITracesTreeHost.SetFolderAutomatic(", "\n    }\n");
        Assert.Equal(2, Count(press, "_tree?.Redraw(folder.Id);"));
        Assert.DoesNotContain("_tree?.Refresh();", press, StringComparison.Ordinal);

        // The queue's refreshes draw once per burst, not once per row added.
        Assert.Equal(0, Count(tree, "CollectionChanged += (_, _) => _tree.Refresh();"));
        Assert.Equal(2, Count(tree, "_tree.RequestRefresh();"));

        string view = ShellSource("TraceCommons.App/Controls/TracesTreeView.cs");
        Assert.Contains("TreeRowKeys.Plan(", view, StringComparison.Ordinal);
        Assert.Contains("TreeRowKeys.Folder(", view, StringComparison.Ordinal);
        Assert.Contains("TreeRowKeys.Session(", view, StringComparison.Ordinal);

        // A kept row's buttons act on the live rows, not the ones it was built with.
        Assert.Contains("_host.SubmitFolder(_host.Group(folder.Id) ?? group, null)", view, StringComparison.Ordinal);
        Assert.Contains("_host.IgnoreFolder(_host.Group(folder.Id) ?? group)", view, StringComparison.Ordinal);
        Assert.Contains("_host.DismissTrace(_host.Entry(entry.EntryId) ?? live)", view, StringComparison.Ordinal);

        string map = ShellSource("TraceCommons.App/Controls/TracesMapView.cs");
        string read = Between(map, "public async Task ReadAsync()", "\n    }\n");
        Assert.Contains("MapSnapshot.SameRead(_snapshot, snapshot)", read, StringComparison.Ordinal);
    }

    private static string Between(string text, string start, string end)
    {
        int from = text.IndexOf(start, StringComparison.Ordinal);
        Assert.True(from >= 0, start);
        int to = text.IndexOf(end, from, StringComparison.Ordinal);
        return text[from..to];
    }

    private static int Count(string text, string needle)
    {
        int n = 0;
        for (int i = text.IndexOf(needle, StringComparison.Ordinal); i >= 0; i = text.IndexOf(needle, i + 1, StringComparison.Ordinal))
        {
            n++;
        }

        return n;
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
