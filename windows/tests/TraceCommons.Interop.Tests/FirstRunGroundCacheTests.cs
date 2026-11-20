using System;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first-run stage's backdrop is its two window-sized gradients (the
/// ground) under a blurred breathing glow. On the GPU-less test box it is
/// most of a frame's cost: the stage drew 10.7 frames a second, 15.8 with
/// the ground left out and 18.1 with the breath left out (an experiment
/// build, tc-win-az7, 2026-11-20). The ground never moves, so it is drawn
/// once per canvas size and DPI into a bitmap and copied each frame; the
/// breath is still drawn every frame.
/// </summary>
public class FirstRunGroundCacheTests
{
    [Fact]
    public void TheGroundIsDrawnAgainOnlyForANewSizeOrDpi()
    {
        var cache = new FirstRunGroundCache();
        Assert.True(cache.Redraw(860, 760, 96));
        Assert.False(cache.Redraw(860, 760, 96));
        Assert.False(cache.Redraw(860, 760, 96));
        Assert.True(cache.Redraw(900, 760, 96));
        Assert.True(cache.Redraw(900, 780, 96));
        Assert.True(cache.Redraw(900, 780, 120));
        Assert.False(cache.Redraw(900, 780, 120));
        cache.Forget();
        Assert.True(cache.Redraw(900, 780, 120));
    }

    [Fact]
    public void ThePainterCopiesTheGroundAndDrawsTheBreathEachFrame()
    {
        string source = System.IO.File.ReadAllText(System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/FirstRun/FirstRunStagePainter.cs.txt"))
            .Replace("\r\n", "\n", StringComparison.Ordinal);
        int at = source.IndexOf("    public void Draw(", StringComparison.Ordinal);
        string draw = source[at..source.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
        Assert.Contains("_groundCache.Redraw(", draw, StringComparison.Ordinal);
        Assert.Contains("ds.DrawImage(_ground, 0, 0)", draw, StringComparison.Ordinal);
        Assert.Contains("Breath(ds, rc, now, reduce)", draw, StringComparison.Ordinal);

        // A stage scaled down into a small window draws its two backdrops as before.
        Assert.Contains("Backdrop(ds, rc, width, height, now, reduce)", draw, StringComparison.Ordinal);
        Assert.Contains("Backdrop(ds, rc, stageWidth, stageHeight, now, reduce)", draw, StringComparison.Ordinal);
    }
}
