using System;
using System.Diagnostics;
using System.IO;

namespace TraceCommons.App.Look;

/// <summary>
/// Lines about the look (the material chosen, the motion setting, the ember,
/// clocks starting and stopping) to Debug output and, when
/// TRACE_COMMONS_LOOK_LOG names a file, to that file, so the test box can
/// read what a run did. "1" writes tc-look.log in the temporary folder.
/// Never a secret or a path of the person's.
/// </summary>
internal static class LookTrace
{
    private static readonly string? Path = Resolve();

    /// <summary>Whether lines go to a file (TRACE_COMMONS_LOOK_LOG is set).</summary>
    public static bool Enabled => !string.IsNullOrEmpty(Path);

    public static void Write(string line)
    {
        Debug.WriteLine("monitor: " + line);
        if (string.IsNullOrEmpty(Path))
        {
            return;
        }

        try
        {
            File.AppendAllText(Path, $"{DateTime.UtcNow:o} {line}{Environment.NewLine}");
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    private static string? Resolve()
    {
        string? path = Environment.GetEnvironmentVariable("TRACE_COMMONS_LOOK_LOG");
        return path == "1" ? System.IO.Path.Combine(System.IO.Path.GetTempPath(), "tc-look.log") : path;
    }
}
