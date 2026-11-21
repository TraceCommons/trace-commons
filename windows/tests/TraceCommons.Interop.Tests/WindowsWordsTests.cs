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
