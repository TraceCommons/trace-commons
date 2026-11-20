using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The flyout opens at the icon the callback names, not where the cursor
/// happens to be: a keyboard select (Win+B, arrows, Enter) leaves the cursor
/// wherever it was, possibly on another monitor (review of #2211, M3).
/// </summary>
public class TrayAnchorTests
{
    private static long Pack(int x, int y) => (long)(((uint)(ushort)(short)y << 16) | (ushort)(short)x);

    [Fact]
    public void TheAnchorIsTheIconsNotTheCursors()
    {
        Assert.Equal((1830, 1060), TrayAnchor.From(Pack(1830, 1060), cursor: (200, 300)));
    }

    [Fact]
    public void AMonitorLeftOfThePrimaryKeepsItsNegativeCoordinates()
    {
        Assert.Equal((-90, 1040), TrayAnchor.From(Pack(-90, 1040), cursor: (500, 500)));
        Assert.Equal((300, -20), TrayAnchor.From(Pack(300, -20), cursor: (500, 500)));
    }
}
