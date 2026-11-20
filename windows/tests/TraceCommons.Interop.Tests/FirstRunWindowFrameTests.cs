using System;
using System.Collections.Generic;
using System.IO;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first-run window's first frame and minimum: wholly on the work area,
/// the Mac's size wherever the display has room.
/// </summary>
public sealed class FirstRunWindowFrameTests
{
    public static IEnumerable<object[]> Frames() => new[]
    {
        // The test box, 2026-11-20: Windows put the 862x762 window at y = 26
        // on a 1024x768 display whose work area ends at 720 (the taskbar).
        new object[] { new LayoutRect(81, 26, 862, 762), 1.0, 2.0, 2.0, new LayoutRect(0, 0, 1024, 720), new LayoutRect(81, 0, 862, 720) },
        // 1920x1080: room enough, so the Mac's size where Windows put it.
        new object[] { new LayoutRect(529, 135, 862, 762), 1.0, 2.0, 2.0, new LayoutRect(0, 0, 1920, 1032), new LayoutRect(529, 135, 862, 762) },
        // Windows' first size is not the canvas: the frame takes the canvas size.
        new object[] { new LayoutRect(200, 100, 640, 480), 1.0, 2.0, 2.0, new LayoutRect(0, 0, 1920, 1032), new LayoutRect(200, 100, 862, 762) },
        // 1024x768 at 125%: 1077 by 952 is held to the work area both ways.
        new object[] { new LayoutRect(40, 20, 1077, 952), 1.25, 2.0, 2.0, new LayoutRect(0, 0, 1024, 720), new LayoutRect(0, 0, 1024, 720) },
        // A second monitor below the first: its own work area's origin counts.
        new object[] { new LayoutRect(100, 1400, 862, 762), 1.0, 2.0, 2.0, new LayoutRect(0, 1080, 1920, 1032), new LayoutRect(100, 1350, 862, 762) },
    };

    [Theory]
    [MemberData(nameof(Frames))]
    public void TheFirstRunWindowIsWhollyOnTheWorkArea(LayoutRect frame, double scale, double extraWidth, double extraHeight, LayoutRect work, LayoutRect want)
    {
        Assert.Equal(want, FirstRunStageLayout.FirstWindowFrame(frame, scale, extraWidth, extraHeight, work));
    }

    [Theory]
    // Room enough: the Mac's minimum, 760 by 640, plus the frame.
    [InlineData(1.0, 1920, 1032, 762, 642)]
    // 1024x768 at 125%: 952 tall would put the window past the 720 work area.
    [InlineData(1.25, 1024, 720, 952, 720)]
    public void TheMinimumNeverExceedsTheWorkArea(double scale, double workWidth, double workHeight, int width, int height)
    {
        Assert.Equal((width, height), FirstRunStageLayout.MinimumWindow(scale, 2, 2, new LayoutRect(0, 0, workWidth, workHeight)));
    }

    [Fact]
    public void TheFirstRunWindowIsSizedWithTheFirstFrame()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/FirstRunWindow.xaml.cs.txt");
        string source = File.ReadAllText(path);
        Assert.Contains("FirstRunStageLayout.FirstWindowFrame(", source);
        Assert.Contains("FirstRunStageLayout.MinimumWindow(", source);
    }
}
