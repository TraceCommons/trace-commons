using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Traces toolbar's "Automatic for every folder" eye and the column
/// titles over the tree, ported from the Mac (SharingChoiceCard.swift
/// <c>SharingChoice</c> and <c>toolbarButton</c>, TracesViews.swift:375-388,
/// :193-198, :1537-1566). Every word is the core's.
/// </summary>
public class SharingChoiceTests
{
    private static string Sample(string set, string method) =>
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "RecordedSamples", set, method + ".json"));

    // ---- What is in force (SharingChoiceCard.swift:18-21) ----------------

    [Fact]
    public void AutomaticIsInForceOnlyWhileTheIncludeExistingGrantIs()
    {
        Assert.Null(SharingChoice.Current(null));
        Assert.Equal(SharingPath.Automatic, SharingChoice.Current(new AutomaticGrantState(true, true)));
        Assert.Equal(SharingPath.AskMe, SharingChoice.Current(new AutomaticGrantState(true, false)));
        Assert.Equal(SharingPath.AskMe, SharingChoice.Current(new AutomaticGrantState(true, null)));
        Assert.Equal(SharingPath.AskMe, SharingChoice.Current(new AutomaticGrantState(false, null)));
    }

    [Fact]
    public void TheGrantDecodesFromTheDaemonsReply()
    {
        Assert.Equal(new AutomaticGrantState(false, null), AutomaticGrantState.Decode(JsonDocument.Parse("""{"granted":false}""").RootElement));
        Assert.Equal(new AutomaticGrantState(true, true), AutomaticGrantState.Decode(JsonDocument.Parse(
            """{"granted":true,"granted_at":"2026-11-01T00:00:00Z","on_disk_recorded":true,"include_existing":true}""").RootElement));
        Assert.Null(AutomaticGrantState.Decode(JsonDocument.Parse("""{"include_existing":true}""").RootElement));
        Assert.Null(AutomaticGrantState.Decode(JsonDocument.Parse("[]").RootElement));
        Assert.Null(AutomaticGrantState.Decode(null));
    }

    [Theory]
    [InlineData("empty")]
    [InlineData("normalDay")]
    [InlineData("armedFolder")]
    [InlineData("heldSessions")]
    public void EverySampleSetRecordsTheGrant(string set)
    {
        // The eye is drawn only once the grant is known; without a recorded
        // reply a sample launch could never show it.
        Assert.NotNull(AutomaticGrantState.Decode(JsonDocument.Parse(Sample(set, "automatic_grant")).RootElement));
    }

    // ---- Folders set to Ask (SharingChoiceCard.swift:24-28) -------------

    [Fact]
    public void AskingCountsConfiguredFoldersSetToAskByARuleOfTheirOwn()
    {
        string json = """
        {"projects":[
          {"project_id":"a","configured":true,"is_unresolved_bucket":false,"mode":"notify_only","folder_mode":"notify_only"},
          {"project_id":"b","configured":true,"is_unresolved_bucket":false,"mode":"auto_upload","folder_mode":"notify_only"},
          {"project_id":"c","configured":true,"is_unresolved_bucket":false,"mode":"notify_only","folder_mode":"auto_upload"},
          {"project_id":"d","configured":false,"is_unresolved_bucket":false,"mode":"notify_only"},
          {"project_id":"e","configured":true,"is_unresolved_bucket":true,"mode":"notify_only"},
          {"project_id":"f","configured":true,"is_unresolved_bucket":false,"mode":"notify_only"},
          {"project_id":"g","configured":true,"is_unresolved_bucket":false,"mode":"ignore"}
        ],"unpurposed_traces":0}
        """;
        // a, b (folder_mode wins) and f (mode when folder_mode is absent).
        Assert.Equal(3, SharingChoice.AskingFolders(JsonDocument.Parse(json).RootElement));
        Assert.Equal(0, SharingChoice.AskingFolders(JsonDocument.Parse("""{"projects":[]}""").RootElement));
        Assert.Equal(0, SharingChoice.AskingFolders(JsonDocument.Parse("{}").RootElement));
    }

    [Fact]
    public void TheRecordedSamplesCountTheirAskFolders()
    {
        Assert.Equal(2, SharingChoice.AskingFolders(JsonDocument.Parse(Sample("normalDay", "list_projects")).RootElement));
        Assert.Equal(0, SharingChoice.AskingFolders(JsonDocument.Parse(Sample("armedFolder", "list_projects")).RootElement));
    }

    [Fact]
    public void TheFirstStepAsksAboutAskFoldersOnlyWhenThereAreSome()
    {
        Assert.Equal(new SharingStep.KeepAsks(2), SharingChoice.FirstStep(2));
        Assert.Equal(new SharingStep.Disclose(true), SharingChoice.FirstStep(0));
    }

    // ---- The eye's tooltip and when it is drawn ------------------------

    [Fact]
    public void TheCountLineCountsAutomaticFoldersOfEveryFolderWithAMode()
    {
        var folders = new[]
        {
            Folder("a", MapModes.AutoUpload), Folder("b", MapModes.Ask), Folder("c", MapModes.Ignore),
            Folder("d", null), Folder("e", MapModes.AutoUpload),
        };
        Assert.Equal("2 of 4", SharingChoice.AutomaticCount(folders, "{count} of {total}"));
    }

    [Fact]
    public void TheTooltipIsTheNameThenTheLine()
    {
        Assert.Equal("Name\nLine", SharingChoice.ToolbarHelp("Name", "Line"));
        Assert.Equal("Name", SharingChoice.ToolbarHelp("Name", ""));
    }

    [Fact]
    public void TheEyeIsDrawnOnlyOnceTheGrantAndItsWordsAreKnown()
    {
        Assert.True(SharingChoice.DrawsToolbarButton(grantKnown: true, wordsKnown: true));
        Assert.False(SharingChoice.DrawsToolbarButton(grantKnown: false, wordsKnown: true));
        Assert.False(SharingChoice.DrawsToolbarButton(grantKnown: true, wordsKnown: false));
    }

    // ---- The column titles (TracesViews.swift:193-198, GTK column_titles) --

    [Fact]
    public void TheColumnTitlesAreTheCoresWordsForEveryComputerButAMac()
    {
        TracesColumnTitles? titles = SharingChoice.ColumnTitles(TreeWords.Core.Value);
        Assert.NotNull(titles);
        Assert.Equal(TreeWords.Core.Value["column_folders_computer"], titles!.Folders);
        Assert.DoesNotContain("Mac", titles.Folders, StringComparison.Ordinal);
        Assert.Equal(TreeWords.Core.Value["column_submit"], titles.Submit);
        Assert.Equal(TreeWords.Core.Value["column_automatic"], titles.Automatic);
        Assert.All(new[] { titles.Folders, titles.Submit, titles.Automatic }, w => Assert.NotEmpty(w));
    }

    [Fact]
    public void NoColumnTitlesWithoutTheEyesWords()
    {
        // The Mac draws them only while the Automatic switches show.
        var words = new Dictionary<string, string>
        {
            ["column_folders_computer"] = "F", ["column_submit"] = "S", ["column_automatic"] = "A",
        };
        var empty = new Dictionary<string, string>();
        Assert.Null(SharingChoice.ColumnTitles(new TreeWords(words, empty, empty)));
        words["automatic_all"] = "All";
        words["automatic_count"] = "{count}";
        Assert.Equal(new TracesColumnTitles("F", "S", "A"), SharingChoice.ColumnTitles(new TreeWords(words, empty, empty)));
        words.Remove("column_submit");
        Assert.Null(SharingChoice.ColumnTitles(new TreeWords(words, empty, empty)));
    }

    // ---- A press (SharingChoiceCard.swift:217-260) -----------------------

    private sealed class Fake : ISharingChoiceSteps
    {
        public readonly List<string> Calls = new();
        public bool? KeepAnswer = true;
        public bool Disclosed = true;
        public Flow1GrantRequest? Request = new(true, Array.Empty<string>(), "witness");
        public GrantAnswer Grant = new(true, null);
        public bool Withdrawn = true;

        public Task<bool?> AskKeepFolderAsks(int count) { Calls.Add($"ask {count}"); return Task.FromResult(KeepAnswer); }

        public Task<SharingDisclosureFlow?> Disclose()
        {
            Calls.Add("disclose");
            if (!Disclosed) return Task.FromResult<SharingDisclosureFlow?>(null);
            var flow = new SharingDisclosureFlow();
            flow.AcknowledgeScrub();
            flow.AcknowledgeWitness("witness");
            return Task.FromResult<SharingDisclosureFlow?>(flow);
        }

        public Task<Flow1GrantRequest?> GrantRequest(SharingDisclosureFlow flow) { Calls.Add("request"); return Task.FromResult(Request); }

        public Task<GrantAnswer> GrantAutomatic(string? witness, bool keepFolderAsks)
        {
            Calls.Add($"grant {witness} keep={keepFolderAsks}");
            return Task.FromResult(Grant);
        }

        public Task<bool> SetAskEachTime() { Calls.Add("ask-each-time"); return Task.FromResult(Withdrawn); }
    }

    private static readonly SharingRefusals Refusals = new("refused", "save-failed");

    [Fact]
    public async Task PressingItOffIsAskEachTime()
    {
        var fake = new Fake();
        Assert.Null(await SharingChoice.PressAsync(SharingPath.Automatic, 3, fake, Refusals));
        Assert.Equal(new[] { "ask-each-time" }, fake.Calls);

        fake = new Fake { Withdrawn = false };
        Assert.Equal("save-failed", await SharingChoice.PressAsync(SharingPath.Automatic, 3, fake, Refusals));
    }

    [Fact]
    public async Task PressingItOnAsksAboutAskFoldersThenDisclosesThenGrants()
    {
        var fake = new Fake { KeepAnswer = false };
        Assert.Null(await SharingChoice.PressAsync(SharingPath.AskMe, 2, fake, Refusals));
        Assert.Equal(new[] { "ask 2", "disclose", "request", "grant witness keep=False" }, fake.Calls);

        fake = new Fake();
        Assert.Null(await SharingChoice.PressAsync(SharingPath.AskMe, 0, fake, Refusals));
        Assert.Equal(new[] { "disclose", "request", "grant witness keep=True" }, fake.Calls);
    }

    [Fact]
    public async Task CancellingAnyStepGrantsNothingAndSaysNothing()
    {
        var fake = new Fake { KeepAnswer = null };
        Assert.Null(await SharingChoice.PressAsync(SharingPath.AskMe, 2, fake, Refusals));
        Assert.Equal(new[] { "ask 2" }, fake.Calls);

        fake = new Fake { Disclosed = false };
        Assert.Null(await SharingChoice.PressAsync(SharingPath.AskMe, 0, fake, Refusals));
        Assert.Equal(new[] { "disclose" }, fake.Calls);
    }

    [Fact]
    public async Task TheCoreDecidesWhetherTheGrantMayBeAskedFor()
    {
        var fake = new Fake { Request = new Flow1GrantRequest(false, new[] { "not-connected" }, null) };
        Assert.Equal("refused", await SharingChoice.PressAsync(SharingPath.AskMe, 0, fake, Refusals));
        Assert.DoesNotContain(fake.Calls, c => c.StartsWith("grant", StringComparison.Ordinal));

        fake = new Fake { Request = null };
        Assert.Equal("refused", await SharingChoice.PressAsync(SharingPath.AskMe, 0, fake, Refusals));
        Assert.DoesNotContain(fake.Calls, c => c.StartsWith("grant", StringComparison.Ordinal));

        fake = new Fake { Grant = new GrantAnswer(false, "automatic-grant-unavailable") };
        Assert.Equal("refused", await SharingChoice.PressAsync(SharingPath.AskMe, 0, fake, Refusals));
    }

    [Fact]
    public async Task NothingIsDoneBeforeTheGrantIsKnown()
    {
        var fake = new Fake();
        Assert.Null(await SharingChoice.PressAsync(null, 2, fake, Refusals));
        Assert.Empty(fake.Calls);
    }

    private static TracesTree.FolderNode Folder(string id, string? mode) => new(id, id, mode, Array.Empty<MapEntry>());
}
