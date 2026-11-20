using System;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first-run stage's backdrop (two window-sized gradients and a blurred
/// breathing ellipse) samples time 30 times a second, but the stage is drawn
/// at 60 on Uses and Traces and at the display's rate while something moves.
/// On a machine with no GPU each backdrop costs a full-window gradient fill
/// and a blur on the CPU. It is now drawn once per phase into a bitmap and
/// copied on the frames in between: the same pixels, half the work at 60.
/// </summary>
public class FirstRunBackdropCacheTests
{
    [Fact]
    public void TheBackdropIsDrawnAgainOnlyForANewPhaseSizeScaleOrDpi()
    {
        var cache = new FirstRunBackdropCache();
        Assert.True(cache.Redraw(860, 760, 1, 96, 0.25));
        Assert.False(cache.Redraw(860, 760, 1, 96, 0.25));
        Assert.True(cache.Redraw(860, 760, 1, 96, 0.26));
        Assert.True(cache.Redraw(900, 760, 1, 96, 0.26));
        Assert.True(cache.Redraw(900, 760, 0.9, 96, 0.26));
        Assert.True(cache.Redraw(900, 760, 0.9, 120, 0.26));
        Assert.False(cache.Redraw(900, 760, 0.9, 120, 0.26));
        cache.Forget();
        Assert.True(cache.Redraw(900, 760, 0.9, 120, 0.26));
    }

    [Fact]
    public void AtSixtyAFrameEveryOtherFrameReusesTheBackdrop()
    {
        var cache = new FirstRunBackdropCache();
        int drawn = 0;
        for (int frame = 0; frame < 120; frame++)
        {
            double now = 1000 + frame * FirstRunMotion.Sixty;
            if (cache.Redraw(860, 760, 1, 96, FirstRunBackdropCache.Phase(now, reduceMotion: false)))
            {
                drawn++;
            }
        }

        Assert.InRange(drawn, 59, 61);
    }

    [Fact]
    public void ThePhaseIsTheBackdropTimelines()
    {
        double now = 12.3456;
        Assert.Equal(SceneMotion.BackdropPhase(FirstRunStageLayout.AtRate(now, FirstRunMotion.Thirty), false), FirstRunBackdropCache.Phase(now, false));
        Assert.Equal(SceneMotion.BackdropPhase(0, true), FirstRunBackdropCache.Phase(now, true));
    }

    [Fact]
    public void ThePainterCopiesTheCachedBackdrop()
    {
        string source = System.IO.File.ReadAllText(System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/FirstRun/FirstRunStagePainter.cs.txt"))
            .Replace("\r\n", "\n", StringComparison.Ordinal);
        int at = source.IndexOf("    public void Draw(", StringComparison.Ordinal);
        string draw = source[at..source.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
        Assert.Contains("_backdropCache.Redraw(", draw, StringComparison.Ordinal);
        Assert.Contains("ds.DrawImage(_backdrop", draw, StringComparison.Ordinal);
        Assert.DoesNotContain("Backdrop(ds,", draw, StringComparison.Ordinal);
    }
}
