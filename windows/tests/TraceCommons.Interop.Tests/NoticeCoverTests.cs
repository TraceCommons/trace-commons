using System;
using System.IO;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// #2275: a control the notice stack draws over is out of the Tab order and
/// the UI Automation tree while it is covered, and back when the notice
/// goes. On Private AI the offer notice's X ("Not now") sat over the guide
/// card's X ("Close"); a pointer reached only "Not now", while Tab and UIA
/// reached both, the covered one included.
/// </summary>
public class NoticeCoverTests
{
    private static readonly CoverRect Card = new(22, 16, 600, 120);

    /// <summary>The geometry read on tc-win-az6 (#2275), in the card's terms: the guide X sits inside the offer card.</summary>
    [Fact]
    public void AControlInsideTheCardIsCovered()
    {
        var guideClose = new CoverRect(520, 40, 20, 20);
        Assert.True(NoticeCover.Covers(Card, guideClose));
    }

    [Fact]
    public void AControlTheCardLeavesShowingIsNotCovered()
    {
        // Wholly below the card.
        Assert.False(NoticeCover.Covers(Card, new CoverRect(520, 200, 20, 20)));
        // Straddling its foot: the pointer still reaches the part below.
        Assert.False(NoticeCover.Covers(Card, new CoverRect(520, 130, 20, 20)));
        // Straddling its side.
        Assert.False(NoticeCover.Covers(Card, new CoverRect(610, 40, 40, 20)));
    }

    [Fact]
    public void NothingIsCoveredByAnEmptyCardOrIsAnEmptyControl()
    {
        Assert.False(NoticeCover.Covers(new CoverRect(22, 16, 0, 0), new CoverRect(22, 16, 0, 0)));
        Assert.False(NoticeCover.Covers(new CoverRect(22, 16, 600, 0), new CoverRect(100, 16, 20, 0)));
        Assert.False(NoticeCover.Covers(Card, new CoverRect(100, 40, 0, 0)));
    }

    /// <summary>The edges themselves count as covered: a control flush with the card's edge has no part showing.</summary>
    [Fact]
    public void AControlFlushWithTheCardIsCovered()
    {
        Assert.True(NoticeCover.Covers(Card, Card));
    }

    [Fact]
    public void TheChangeHidesNewlyCoveredAndRestoresUncovered()
    {
        NoticeCoverChange<string> first = NoticeCover.Change(Array.Empty<string>(), new[] { "guide-close" });
        Assert.Equal(new[] { "guide-close" }, first.Hide);
        Assert.Empty(first.Restore);

        // Still covered: nothing to do, so the saved state is not overwritten.
        NoticeCoverChange<string> same = NoticeCover.Change(new[] { "guide-close" }, new[] { "guide-close" });
        Assert.Empty(same.Hide);
        Assert.Empty(same.Restore);

        // The notice went: the guide's X comes back.
        NoticeCoverChange<string> gone = NoticeCover.Change(new[] { "guide-close" }, Array.Empty<string>());
        Assert.Empty(gone.Hide);
        Assert.Equal(new[] { "guide-close" }, gone.Restore);

        // A taller card covers one more; a scroll uncovers another.
        NoticeCoverChange<string> moved = NoticeCover.Change(new[] { "a", "b" }, new[] { "b", "c" });
        Assert.Equal(new[] { "c" }, moved.Hide);
        Assert.Equal(new[] { "a" }, moved.Restore);
    }

    /// <summary>
    /// Focus that lands on a control the card then covers (Tab brought it into
    /// view, under the card) moves on the way it was going, so a Tab press is
    /// never spent on a covered control; focus that arrived another way goes to
    /// the card's X, the control drawn in its place.
    /// </summary>
    [Fact]
    public void FocusOnACoveredControlMovesTheWayItWasGoing()
    {
        Assert.Equal(CoverFocusMove.Forward, NoticeCover.FocusMove(tabbedForward: true));
        Assert.Equal(CoverFocusMove.Back, NoticeCover.FocusMove(tabbedForward: false));
        Assert.Equal(CoverFocusMove.ToCard, NoticeCover.FocusMove(tabbedForward: null));
    }

    /// <summary>
    /// The window plays the decision: read as source, since the suite runs on
    /// macOS where no WinUI element can be made. The behaviour is witnessed on
    /// the box (Tab walk and UIA dump of Private AI with and without the notice).
    /// </summary>
    [Fact]
    public void TheWindowTakesCoveredControlsOutOfTabAndUia()
    {
        string source = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.NoticeCover.cs.txt"));
        Assert.Contains("NoticeCover.Covers(", source, StringComparison.Ordinal);
        Assert.Contains("NoticeCover.Change(", source, StringComparison.Ordinal);
        Assert.Contains("IsTabStop = false", source, StringComparison.Ordinal);
        Assert.Contains("AccessibilityView.Raw", source, StringComparison.Ordinal);
        Assert.Contains("FindElementsInHostCoordinates", source, StringComparison.Ordinal);
        // A covered control that held focus passes it on.
        Assert.Contains("NoticeCover.FocusMove(", source, StringComparison.Ordinal);
        Assert.Contains("GettingFocus", source, StringComparison.Ordinal);

        // Re-read whenever the notices change, the card resizes, or the tab scrolls under it.
        string shell = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.Shell.cs.txt"));
        int refresh = shell.IndexOf("private void RefreshNotices()", StringComparison.Ordinal);
        Assert.True(refresh >= 0);
        string refreshBody = shell.Substring(refresh, shell.IndexOf("private NoticePlace NoticeMonitorPlace()", refresh, StringComparison.Ordinal) - refresh);
        Assert.True(refreshBody.Split("UpdateNoticeCover()").Length - 1 >= 2, "both of RefreshNotices' exits re-read the cover");
        Assert.Contains("ViewChanged", source, StringComparison.Ordinal);
        Assert.Contains("RefreshPlatesOnly();", shell, StringComparison.Ordinal);
        int plates = shell.IndexOf("private void RefreshPlatesOnly()", StringComparison.Ordinal);
        Assert.Contains("UpdateNoticeCover()", shell.Substring(plates, 400), StringComparison.Ordinal);
    }
}
