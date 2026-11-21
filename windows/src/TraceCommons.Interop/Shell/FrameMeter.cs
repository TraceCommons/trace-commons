using System;
using System.Collections.Generic;
using System.Globalization;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// Counts a drawn surface's frames for the look log: once a second per
/// surface, a line with how many frames it drew, the rate, the mean and
/// longest time its draw handler took, and the longest gap between two
/// frames. The handler's time leaves out the rasterising Direct2D does after
/// it returns, so the rate is the measure of a slow frame. Read by the CPU
/// profile (windows/scripts/perf); written only when TRACE_COMMONS_LOOK_LOG
/// names a file.
/// </summary>
public sealed class FrameMeter
{
    private sealed class Window
    {
        public double Start, Last = -1, Sum, Max, MaxGap;
        public int N;
    }

    private readonly Dictionary<string, Window> _windows = new();

    /// <summary>One frame of <paramref name="surface"/> at <paramref name="now"/> (seconds); a line when a second has passed.</summary>
    public string? Add(string surface, double now, double drawMs)
    {
        if (!_windows.TryGetValue(surface, out Window? w))
        {
            w = new Window { Start = now };
            _windows[surface] = w;
        }

        if (w.Last >= 0)
        {
            w.MaxGap = Math.Max(w.MaxGap, now - w.Last);
        }

        w.Last = now;
        w.N++;
        w.Sum += drawMs;
        w.Max = Math.Max(w.Max, drawMs);
        double over = now - w.Start;
        if (over < 1 - 1e-9)
        {
            return null;
        }

        string line = string.Format(CultureInfo.InvariantCulture,
            "frames {0} n={1} over={2:0.000}s fps={3:0.0} draw_mean={4:0.00}ms draw_max={5:0.00}ms gap_max={6:0.0}ms",
            surface, w.N, over, w.N / over, w.Sum / w.N, w.Max, w.MaxGap * 1000);
        w.Start = now;
        w.N = 0;
        w.Sum = 0;
        w.Max = 0;
        w.MaxGap = 0;
        return line;
    }
}
