using System;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// One gate for a trace's Submit wherever it is drawn (TracesStore.swift:801-806,
/// <c>maySubmitSession</c>, which the Mac's tree and its rail both ask): the
/// core answered and is signed in, the trace's folder is not ignored, the
/// trace is not held for review, and the shared table offers Contribute.
/// The inspector's trace cards used to offer Submit on the last alone.
/// </summary>
public class TraceSubmitGateTests
{
    private static async Task<TracesTreeState> ReadState(string set, bool showIgnored = false)
    {
        var state = new TracesTreeState();
        state.ShowIgnored(showIgnored);
        state.Read(await FlowMapSceneTests.Read(set), MapFeed.CoreScansWhenUnset);
        return state;
    }

    [Fact]
    public async Task ASignedInTraceInAnOfferedFolderMaySubmit()
    {
        TracesTreeState state = await ReadState("normalDay");
        TracesTree.FolderNode folder = state.Tree.Folders.First(f => f.Mode != MapModes.Ignore && f.Sessions.Count > 0);
        string entry = folder.Sessions[0].EntryId;

        Assert.True(state.MaySubmit(entry, offersContribute: true, "pending", null));
        Assert.False(state.MaySubmit(entry, offersContribute: false, "pending", null));
        Assert.False(state.MaySubmit(entry, offersContribute: true, "pending", "witness-risk-review-required"));
    }

    [Fact]
    public void NothingMaySubmitBeforeTheCoreAnswers()
    {
        Assert.False(new TracesTreeState().MaySubmit("any", offersContribute: true, "pending", null));
    }

    [Fact]
    public void ATraceInAnIgnoredFolderMayNotSubmit()
    {
        // No recorded set holds a trace in an ignored folder (ignoring a
        // folder purges what waits in it), so the tree is built here.
        var entry = new MapEntry("e1", "claude_code", null, "p1", "repo", DateTimeOffset.UnixEpoch, null);
        TracesTree Tree(string mode) => new(
            Array.Empty<TracesTree.ToolNode>(), Array.Empty<TracesTree.FolderNode>(),
            new[] { new TracesTree.FolderNode("p1", "repo", mode, new[] { entry }) });

        Assert.True(TracesTreeRows.MaySubmitIn(Tree(MapModes.Ask), true, true, "e1", true, "pending", null));
        Assert.False(TracesTreeRows.MaySubmitIn(Tree(MapModes.Ignore), true, true, "e1", true, "pending", null));
    }

    /// <summary>The inspector's trace cards draw and send Submit through the gate.</summary>
    [Fact]
    public void TheInspectorsTraceCardsAskTheGate()
    {
        string xaml = ShellSource("TraceCommons.App/MainWindow.xaml");
        int click = xaml.IndexOf("Click=\"OnSubmitEntry\"", StringComparison.Ordinal);
        Assert.True(click >= 0, "the cards' Submit is gone");
        string button = xaml[xaml.LastIndexOf("<Button", click, StringComparison.Ordinal)..click];
        Assert.Contains("Visibility=\"{x:Bind MaySubmit, Mode=OneWay}\"", button, StringComparison.Ordinal);

        string code = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        int handler = code.IndexOf(" void OnSubmitEntry(", StringComparison.Ordinal);
        Assert.True(handler >= 0, "OnSubmitEntry is gone");
        string body = code[handler..code.IndexOf("\n    }\n", handler, StringComparison.Ordinal)];
        Assert.DoesNotContain("SubmitEntryAsync", body, StringComparison.Ordinal);
        Assert.Contains("SubmitTrace(", body, StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
