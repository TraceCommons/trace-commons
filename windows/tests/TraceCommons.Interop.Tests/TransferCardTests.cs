using System;
using System.Linq;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The card for a trace being contributed, kept or dismissed
/// (<c>TraceTransferCard</c>, TraceTransferCard.swift; W4 "Transfer cards
/// at the map foot", approved as a class 2026-11-19). Every word is the
/// core's; the GTK port (transfer_card.rs) is the precedent for this
/// shell's "this computer" lines.
/// </summary>
public class TransferCardTests
{
    private static FlowMapCopy Words => FlowMapCopy.Table ?? throw new InvalidOperationException("no flow_map table");

    private static MapTransfer One(string tool, TransferAction action) => new("t1", "f1", tool, action);

    [Fact]
    public void EachActionHasTheMacsTagToneAndNote()
    {
        // TraceTransferCard.swift:72-81.
        TransferCardFace contribute = TransferCardFace.For(One("codex", TransferAction.Contribute), Words, null, null);
        Assert.Equal(Words.TransferContributing, contribute.Tag);
        Assert.True(contribute.TagOn);
        Assert.Equal(Words.TransferNote(true, "Codex"), contribute.Note);
        Assert.Contains("Codex", contribute.Note, StringComparison.Ordinal);

        TransferCardFace keep = TransferCardFace.For(One("codex", TransferAction.Keep), Words, null, null);
        Assert.Equal(Words.TransferKeeping, keep.Tag);
        Assert.False(keep.TagOn);
        Assert.Equal(Words["transfer_keep_note_computer"], keep.Note);

        TransferCardFace dismiss = TransferCardFace.For(One("codex", TransferAction.Dismiss), Words, null, null);
        Assert.Equal(Words.TransferDismissed, dismiss.Tag);
        Assert.False(dismiss.TagOn);
        Assert.Equal(Words.TransferDismissNote, dismiss.Note);

        foreach (TransferCardFace face in new[] { contribute, keep, dismiss })
        {
            Assert.False(string.IsNullOrEmpty(face.Tag));
            Assert.False(string.IsNullOrEmpty(face.Note));
        }
    }

    [Fact]
    public void ABatchFromSeveralToolsNamesNone()
    {
        var batch = One("codex", TransferAction.Contribute) with { Count = 3, MixedTools = true, MixedFolders = true };
        TransferCardFace face = TransferCardFace.For(batch, Words, null, null);
        // Its count is the title and the eyebrow; no tool's mark, memory or note.
        Assert.Equal(Words.TransferTraces(3), face.Title);
        Assert.Equal(Words.TransferTraces(3).ToUpperInvariant(), face.Eyebrow);
        Assert.Null(face.Mark);
        Assert.Null(face.MemoryFiles);
        Assert.Null(face.MemoryStays);
        Assert.Equal(Words.TransferContributeNoteMixed, face.Note);
        // Folders differ: none named.
        Assert.Null(face.Folder);
    }

    [Fact]
    public void OneToolsTraceNamesItsToolFolderTimeAndMemory()
    {
        var at = new DateTimeOffset(2026, 11, 19, 15, 4, 0, TimeSpan.Zero);
        var transfer = One("claude-code", TransferAction.Contribute) with { At = at };
        TransferCardFace face = TransferCardFace.For(transfer, Words, "tc-app", TimeZoneInfo.Utc, System.Globalization.CultureInfo.GetCultureInfo("en-US"));
        Assert.Equal("Claude Code", face.Title);
        Assert.Equal(Words.TransferTrace.ToUpperInvariant(), face.Eyebrow);
        Assert.Equal(GlassToolKind.ClaudeCode, face.Mark!.Kind);
        // The core's name, else the map's, else the id (:110).
        Assert.Equal("tc-app", face.Folder);
        Assert.Equal("f-core", TransferCardFace.For(transfer with { FolderLabel = "f-core" }, Words, "tc-app", TimeZoneInfo.Utc).Folder);
        Assert.Equal("f1", TransferCardFace.For(transfer, Words, null, TimeZoneInfo.Utc).Folder);
        // date: .abbreviated, time: .shortened (:113), in the person's zone
        // and culture; no joining word, which would be a new word.
        Assert.Equal("Nov 19, 2026, " + at.ToString("t", System.Globalization.CultureInfo.GetCultureInfo("en-US")), face.Time);
        Assert.StartsWith("Nov 19, 2026, 3:04", face.Time, StringComparison.Ordinal);
        Assert.Null(TransferCardFace.For(transfer with { At = null }, Words, null, TimeZoneInfo.Utc).Time);
        Assert.Equal(Words.MemoryFiles("claude-code"), face.MemoryFiles);
        Assert.Equal(Words.MemoryStays, face.MemoryStays);
        Assert.Equal("Claude Code", face.AccessibleName);
    }

    [Fact]
    public void AnUnknownToolGetsTwoLettersAndNoMemoryFiles()
    {
        TransferCardFace face = TransferCardFace.For(One("aider", TransferAction.Keep), Words, null, null);
        Assert.Equal("aider", face.Title);
        Assert.Equal(GlassToolKind.Other, face.Mark!.Kind);
        Assert.Null(face.MemoryFiles);
        Assert.Equal(Words.MemoryStays, face.MemoryStays);
    }

    [Fact]
    public void TracesShowsTheCardsOverItsListAndTheMapOnlyOverAnotherTab()
    {
        // MonitorNotices.swift:157-161; MonitorWindowView.swift:449 and
        // FlowMapView.swift:176: Traces and History stack them with their
        // notices, in front; the map's foot carries them only while another
        // tab is shown, and only on the Traces map.
        Assert.False(TransferCardFace.OnMap(tracesTabShown: true, mapShowsTraces: true, cards: 2));
        Assert.True(TransferCardFace.OnMap(tracesTabShown: false, mapShowsTraces: true, cards: 2));
        Assert.False(TransferCardFace.OnMap(tracesTabShown: false, mapShowsTraces: false, cards: 2));
        Assert.False(TransferCardFace.OnMap(tracesTabShown: false, mapShowsTraces: true, cards: 0));

        NoticeKind card = NoticeCatalog.Transfer("t1");
        Assert.Equal("transfer:t1", card.Id);
        Assert.Equal(NoticeClose.PutsAway, card.Close);
        var present = new[] { NoticeCatalog.ActionNotice, card, NoticeCatalog.PrivateAIOffer };
        Assert.Equal(new[] { "action-notice", "transfer:t1" }, NoticeCatalog.Order(NoticePlace.Traces, present).Select(c => c.Id));
        Assert.Equal(new[] { "action-notice", "transfer:t1" }, NoticeCatalog.Order(NoticePlace.History, present).Select(c => c.Id));
        Assert.DoesNotContain("transfer:t1", NoticeCatalog.Order(NoticePlace.PrivateAI, present).Select(c => c.Id));
        Assert.True(NoticeCatalog.IsTransfer("transfer:t1"));
        Assert.Equal("t1", NoticeCatalog.TransferId("transfer:t1"));
    }

    [Fact]
    public void ItsDetailsAreNamedWithTheCoresWord()
    {
        Assert.Equal("Details", MonitorWords.NoticeDetails);
    }

    [Fact]
    public void TheWindowShowsTheCardsWhereTheMacDoes()
    {
        string transfers = ShellSource("TraceCommons.App/MainWindow.Transfers.cs");
        string shell = ShellSource("TraceCommons.App/MainWindow.Shell.cs");
        string card = ShellSource("TraceCommons.App/Controls/TransferCardView.cs");
        Assert.Contains("TransferCardFace.OnMap(", transfers, StringComparison.Ordinal);
        Assert.Contains("NoticeCatalog.Transfer(", transfers, StringComparison.Ordinal);
        Assert.Contains("StackAnchor.Bottom", transfers, StringComparison.Ordinal);
        Assert.Contains("TransferNotices()", shell, StringComparison.Ordinal);
        Assert.Contains("CloseTransfer(NoticeCatalog.TransferId(", shell, StringComparison.Ordinal);
        Assert.Contains("RefreshMapTransfers();", shell, StringComparison.Ordinal);
        // Every word on the card is the face's, the core's: none typed here.
        Assert.Contains("MonitorWords.NoticeDetails", card, StringComparison.Ordinal);
        Assert.Contains("MonitorWords.Close", card, StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(System.IO.File.Exists(path), $"{relativePath} was not copied to {path}");
        return System.IO.File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
