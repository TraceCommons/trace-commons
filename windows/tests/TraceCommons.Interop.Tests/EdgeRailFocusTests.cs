using System;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The rail's keyboard and placement rules the box found broken on
/// tc-win-az5 (review of #2211, M2 and M3).
/// </summary>
public class EdgeRailFocusTests
{
    private static readonly IntPtr Rail = new(0x1010);
    private static readonly IntPtr Main = new(0x2020);
    private static readonly IntPtr Editor = new(0x3030);

    /// <summary>A press on the closed zone activates the rail before it opens: the keyboard still goes back to the window before it.</summary>
    [Fact]
    public void APressThatOpensTheRailHandsTheKeyboardBackToTheWindowBefore()
    {
        var focus = new EdgeRailFocus(Rail);
        focus.Foreground(Main);
        focus.Foreground(Rail);
        focus.Opening(foregroundNow: Rail);
        Assert.Equal(Main, focus.HandBack());
    }

    [Fact]
    public void TheLatestOtherWindowIsTheOneHandedBack()
    {
        var focus = new EdgeRailFocus(Rail);
        focus.Foreground(Main);
        focus.Foreground(Editor);
        focus.Opening(foregroundNow: Editor);
        Assert.Equal(Editor, focus.HandBack());
        Assert.Equal(IntPtr.Zero, focus.HandBack());
    }

    /// <summary>After Escape, the pointer still resting where the zone is reopened the rail 120-190 ms later on the box.</summary>
    [Fact]
    public void AfterEscapeThePointerMustLeaveBeforeTheRailReopens()
    {
        var rail = new EdgeRailHover();
        Assert.Equal(EdgeRailStep.GrowThenOpen, rail.PointerEntered(cursorInside: true));
        rail.Clicked();
        Assert.Equal(EdgeRailStep.CloseThenShrink, rail.CloseNow());
        Assert.Equal(EdgeRailStep.None, rail.PointerEntered(cursorInside: true));
        rail.PointerExited(DateTimeOffset.UnixEpoch);
        Assert.Equal(EdgeRailStep.GrowThenOpen, rail.PointerEntered(cursorInside: true));
    }

    /// <summary>A cursor seen away from the zone after Escape ends the wait, so the next real hover opens.</summary>
    [Fact]
    public void ACursorSeenElsewhereAfterEscapeEndsTheWait()
    {
        var rail = new EdgeRailHover();
        rail.PointerEntered(cursorInside: true);
        rail.CloseNow();
        Assert.Equal(EdgeRailStep.None, rail.PointerEntered(cursorInside: false));
        Assert.Equal(EdgeRailStep.GrowThenOpen, rail.PointerEntered(cursorInside: true));
    }

    /// <summary>
    /// A press on the closed zone opens the rail even after Escape and with no
    /// enter seen: on tc-win-az10, 2026-11-19, WinUI still counted the pointer
    /// inside after the rail shrank under it, sent no new enter, and a click
    /// on the zone activated the rail without opening it.
    /// </summary>
    [Fact]
    public void APressOnTheZoneOpensTheRail()
    {
        var rail = new EdgeRailHover();
        rail.PointerEntered(cursorInside: true);
        rail.CloseNow();
        Assert.Equal(EdgeRailStep.GrowThenOpen, rail.Pressed());
        Assert.True(rail.HoldsKeyboard);
        Assert.Equal(EdgeRailStep.None, rail.Pressed());
    }

    /// <summary>WinUI can report an enter from a stale pointer position; the cursor is not on the zone, so nothing opens.</summary>
    [Fact]
    public void AnEnterWithTheCursorElsewhereOpensNothing()
    {
        var rail = new EdgeRailHover();
        Assert.Equal(EdgeRailStep.None, rail.PointerEntered(cursorInside: false));
        Assert.False(rail.Open);
    }

    [Theory]
    [InlineData(0x007Eu, 0L, true)]   // WM_DISPLAYCHANGE
    [InlineData(0x02E0u, 0L, true)]   // WM_DPICHANGED
    [InlineData(0x001Au, 0x2FL, true)] // WM_SETTINGCHANGE, SPI_SETWORKAREA
    [InlineData(0x001Au, 0x1043L, false)] // WM_SETTINGCHANGE, SPI_SETCLIENTAREAANIMATION
    [InlineData(0x0200u, 0L, false)]  // WM_MOUSEMOVE
    public void TheRailIsPlacedAgainWhenTheScreenMoves(uint message, long wParam, bool replace)
    {
        Assert.Equal(replace, EdgeRailReplace.After(message, wParam));
    }
}
