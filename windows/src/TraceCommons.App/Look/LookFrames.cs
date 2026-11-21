using System.Diagnostics;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// Frame lines for the look log (<see cref="FrameMeter"/>): a surface's draw
/// handler calls <see cref="Begin"/> and <see cref="End"/>. Nothing is timed
/// or kept unless TRACE_COMMONS_LOOK_LOG names a file; the CPU profile
/// (windows/scripts/perf) sets it to read each surface's frame rate.
/// </summary>
internal static class LookFrames
{
    private static readonly FrameMeter Meter = new();

    /// <summary>A timestamp to pass to <see cref="End"/>, or 0 when the log is off.</summary>
    public static long Begin() => LookTrace.Enabled ? Stopwatch.GetTimestamp() : 0;

    /// <summary>One frame of <paramref name="surface"/>, begun at <paramref name="begun"/>.</summary>
    public static void End(string surface, long begun)
    {
        if (begun == 0)
        {
            return;
        }

        long now = Stopwatch.GetTimestamp();
        double ms = (now - begun) * 1000.0 / Stopwatch.Frequency;
        if (Meter.Add(surface, now / (double)Stopwatch.Frequency, ms) is { } line)
        {
            LookTrace.Write(line);
        }
    }
}
