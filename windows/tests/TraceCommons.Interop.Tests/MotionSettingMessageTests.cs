using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Reduce Motion followed live in both directions (checklist P1). On the
/// test box the map's clock stopped when client-area animation was turned
/// off and stayed stopped when it was turned back on: 0 ms of CPU over 10 s
/// with flowing arcs on screen. The window's settings message is what tells
/// the map to look again.
/// </summary>
public class MotionSettingMessageTests
{
    [Theory]
    [InlineData(0x001Au, 0x1043ul, true)]  // SPI_SETCLIENTAREAANIMATION
    [InlineData(0x001Au, 0x0049ul, true)]  // SPI_SETANIMATION
    [InlineData(0x001Au, 0x0014ul, false)] // SPI_SETDESKWALLPAPER
    [InlineData(0x001Au, 0x009Ful, false)] // SPI_SETLOGICALDPIOVERRIDE
    [InlineData(0x0024u, 0x1043ul, false)] // WM_GETMINMAXINFO
    public void OnlyTheAnimationSwitchesAskTheMapToLookAgain(uint message, ulong wParam, bool expected)
    {
        Assert.Equal(expected, MotionSettingMessage.MayChangeMotion(message, wParam));
    }
}
