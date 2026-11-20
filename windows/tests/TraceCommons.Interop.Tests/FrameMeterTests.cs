using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The look log's frame lines (TRACE_COMMONS_LOOK_LOG): one line a second
/// per drawn surface, with how many frames it drew, the rate, the time its
/// draw handler took and the longest gap between two frames. The CPU profile
/// (windows/scripts/perf) reads them as each surface's frame rate.
/// </summary>
public class FrameMeterTests
{
    [Fact]
    public void ASurfaceGetsOneLineASecond()
    {
        var meter = new FrameMeter();
        var at = new System.Collections.Generic.List<int>();
        string? last = null;
        for (int i = 0; i <= 60; i++)
        {
            if (meter.Add("stage", 100 + i / 30.0, 2) is { } line)
            {
                at.Add(i);
                last = line;
            }
        }

        Assert.Equal(new[] { 30, 60 }, at);
        Assert.Equal("frames stage n=30 over=1.000s fps=30.0 draw_mean=2.00ms draw_max=2.00ms gap_max=33.3ms", last);
    }

    [Fact]
    public void EachSurfaceIsCountedApartAndTheLongestGapIsKept()
    {
        var meter = new FrameMeter();
        Assert.Null(meter.Add("map", 0, 1));
        Assert.Null(meter.Add("still", 0.1, 9));
        Assert.Null(meter.Add("map", 0.5, 3));
        string? line = meter.Add("map", 1.0, 2);
        Assert.Equal("frames map n=3 over=1.000s fps=3.0 draw_mean=2.00ms draw_max=3.00ms gap_max=500.0ms", line);
        Assert.Null(meter.Add("map", 1.2, 1));
        Assert.StartsWith("frames still n=2 ", meter.Add("still", 1.1, 1));
    }
}
