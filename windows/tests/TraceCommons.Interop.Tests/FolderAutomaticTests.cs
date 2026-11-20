using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A folder row's Automatic switch (TracesViews.swift:438-451,
/// <c>automaticSwitch</c>; :685-691, <c>onAutomatic</c>; TracesStore.swift:1039-1061,
/// <c>setFolderMode</c>): on at Automatic, off at Ask, when the core offers
/// the other; none at Never or with no mode; the bucket's stays off. A press
/// is the arming offer's write, <c>set_project_mode</c>, with no
/// confirmation (owner, 2026-11-11), one write per folder at a time; a
/// refusal keeps the core's <c>project_mode_failed</c> line beside the
/// folder and re-reads nothing, a confirmed write re-reads.
/// </summary>
public class FolderAutomaticTests
{
    private const string Ok = """{"id":1,"result":{"ok":true,"purged":0}}""";
    private const string Refused = """{"id":1,"error":{"code":"unavailable","message":"consent-required"}}""";
    private const string Failed = "Couldn't change this folder's setting.";

    private static TracesTree.FolderNode Folder(string? mode, params string[] offered) =>
        new("p1", "repo", mode, Array.Empty<MapEntry>()) { OfferableModes = offered };

    [Fact]
    public void TheSwitchIsTheMacs()
    {
        Assert.True(FolderAutomatic.Switch(Folder(MapModes.AutoUpload, MapModes.Ask, MapModes.Ignore)));
        Assert.False(FolderAutomatic.Switch(Folder(MapModes.Ask, MapModes.AutoUpload, MapModes.Ignore)));
        Assert.Null(FolderAutomatic.Switch(Folder(MapModes.AutoUpload, MapModes.Ignore)));
        Assert.Null(FolderAutomatic.Switch(Folder(MapModes.Ask, MapModes.Ignore)));
        Assert.Null(FolderAutomatic.Switch(Folder(MapModes.Ignore, MapModes.Ask, MapModes.AutoUpload)));
        Assert.Null(FolderAutomatic.Switch(Folder(null, MapModes.Ask, MapModes.AutoUpload)));
        Assert.False(FolderAutomatic.Switch(Folder(MapModes.Ask) with { IsBucket = true }));
        Assert.Null(FolderAutomatic.Switch(Folder(null) with { IsBucket = true }));
    }

    private sealed class Rig
    {
        public readonly List<(string Method, string Params)> Calls = new();
        public readonly Queue<TaskCompletionSource<DaemonResponse>> Replies = new();
        public int Refreshes;
        public readonly FolderAutomatic Writes;

        public Rig()
        {
            Writes = new FolderAutomatic(
                (method, parameters) =>
                {
                    Calls.Add((method, parameters));
                    var reply = new TaskCompletionSource<DaemonResponse>();
                    Replies.Enqueue(reply);
                    return reply.Task;
                },
                () =>
                {
                    Refreshes++;
                    return Task.CompletedTask;
                },
                () => Failed);
        }

        public void Answer(string frame) => Replies.Dequeue().SetResult(DaemonResponse.Parse(frame));
    }

    [Fact]
    public async Task OnArmsTheFolderAndReReads()
    {
        var rig = new Rig();
        Task press = rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.AutoUpload, MapModes.Ignore), true);
        Assert.Equal(DaemonProtocol.Methods.SetProjectMode, rig.Calls[0].Method);
        Assert.Equal("""{"project_id":"p1","mode":"auto_upload"}""", rig.Calls[0].Params);
        Assert.Contains("p1", rig.Writes.Writing);

        rig.Answer(Ok);
        await press;
        Assert.Equal(1, rig.Refreshes);
        Assert.DoesNotContain("p1", rig.Writes.Writing);
        Assert.Null(rig.Writes.Refusal("p1"));
    }

    [Fact]
    public async Task OffAsksFirst()
    {
        var rig = new Rig();
        Task press = rig.Writes.SetAsync(Folder(MapModes.AutoUpload, MapModes.Ask), false);
        Assert.Equal("""{"project_id":"p1","mode":"notify_only"}""", rig.Calls[0].Params);
        rig.Answer(Ok);
        await press;
    }

    [Fact]
    public async Task ARefusalKeepsTheCoresLineAndReReadsNothing()
    {
        var rig = new Rig();
        Task press = rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.AutoUpload), true);
        rig.Answer(Refused);
        await press;
        Assert.Equal(Failed, rig.Writes.Refusal("p1"));
        Assert.Equal(0, rig.Refreshes);
        Assert.DoesNotContain("p1", rig.Writes.Writing);
    }

    [Fact]
    public async Task OneWritePerFolderAtATimeAndNothingForAMoveThatChangesNothing()
    {
        var rig = new Rig();
        Task first = rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.AutoUpload), true);
        await rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.AutoUpload), true);
        Assert.Single(rig.Calls);
        rig.Answer(Ok);
        await first;

        await rig.Writes.SetAsync(Folder(MapModes.AutoUpload, MapModes.Ask), true);
        await rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.Ignore), true);
        Assert.Single(rig.Calls);
    }

    [Fact]
    public async Task TheBucketIsNeverWritten()
    {
        var rig = new Rig();
        await rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.AutoUpload) with { IsBucket = true }, true);
        Assert.Empty(rig.Calls);
    }

    [Fact]
    public async Task AThrownCallIsARefusal()
    {
        var rig = new Rig();
        Task press = rig.Writes.SetAsync(Folder(MapModes.Ask, MapModes.AutoUpload), true);
        rig.Replies.Dequeue().SetException(new IOException("pipe closed"));
        await press;
        Assert.Equal(Failed, rig.Writes.Refusal("p1"));
    }

    /// <summary>The tree draws the switch from these rules and sends a press through them.</summary>
    [Fact]
    public void TheTreeDrawsAndSendsTheSwitch()
    {
        string tree = ShellSource("TraceCommons.App/Controls/TracesTreeView.cs");
        Assert.Contains("FolderAutomatic.Switch(folder)", tree, StringComparison.Ordinal);
        Assert.Contains("_words[\"automatic_folder\"]", tree, StringComparison.Ordinal);
        Assert.Contains("_host.SetFolderAutomatic(", tree, StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
