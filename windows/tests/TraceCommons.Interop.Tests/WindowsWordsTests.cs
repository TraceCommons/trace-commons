using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings' credit-arrival switch on Windows says the taskbar and tray, in
/// the owner's words of 2026-11-19, and never the Mac's Dock and menu bar.
/// </summary>
public class WindowsWordsTests
{
    [Fact]
    public void TheCreditSwitchNamesTheTaskbarAndTray()
    {
        Assert.Equal("Animate taskbar and tray icons", WindowsWords.CreditIconAnimationToggle);
        Assert.Equal(
            "Burst gold coins on the tray icon and flag the taskbar when credits arrive.",
            WindowsWords.CreditIconAnimationCaption);
    }

    /// <summary>
    /// The Look inside window's title word, a key of its own beside the
    /// button's (copy guide rule 4); the word the window has always had.
    /// </summary>
    [Fact]
    public void ThePreviewWindowHasItsOwnTitleWord()
    {
        Assert.Equal("Look inside", WindowsWords.PreviewWindowTitle);
    }

    /// <summary>The owner's words for Insights' breakdown note on a PC (2026-11-21).</summary>
    [Fact]
    public void TheBreakdownNoteSaysThisPc()
    {
        Assert.Equal("This week, from this PC", WindowsWords.InsightsCreditScope);
    }

    [Fact]
    public void TheBreakdownReadsTheWindowsNoteAndNotTheMacs()
    {
        string source = System.IO.File.ReadAllText(System.IO.Path.Combine(
            System.AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "InsightsView.UsageCards.cs.txt"));

        Assert.Contains("WindowsWords.InsightsCreditScope", source);
        Assert.DoesNotContain("\"analytics_credit_scope\"", source);
    }

    /// <summary>
    /// The inspector card is a fixed width and its three buttons do not fit
    /// on one line, so its action row wraps instead of clipping Submit.
    /// </summary>
    [Fact]
    public void TheTraceCardsActionsWrap()
    {
        string xaml = System.IO.File.ReadAllText(System.IO.Path.Combine(
            System.AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.txt"));
        int look = xaml.IndexOf("Click=\"OnLookInside\"", System.StringComparison.Ordinal);
        Assert.True(look > 0);
        string before = xaml.Substring(0, look);

        Assert.True(
            before.LastIndexOf("<controls:FlowPanel", System.StringComparison.Ordinal)
                > before.LastIndexOf("<StackPanel Orientation=\"Horizontal\"", System.StringComparison.Ordinal),
            "the card's action row is a StackPanel, which clips what does not fit");
    }

    [Fact]
    public void NoWindowsWordNamesTheDockOrTheMenuBar()
    {
        foreach (string word in new[] { WindowsWords.CreditIconAnimationToggle, WindowsWords.CreditIconAnimationCaption })
        {
            Assert.DoesNotContain("Dock", word);
            Assert.DoesNotContain("menu bar", word);
        }
    }
}
