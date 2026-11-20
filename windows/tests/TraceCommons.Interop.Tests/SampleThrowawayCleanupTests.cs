using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A sample launch's throwaway state directory
/// (<see cref="SampleDaemon.ThrowawayConfigDir"/>) is removed when the
/// launch exits, and a launch removes those left by sample launches that
/// were force-stopped. Before this, opening Settings in a sample launch
/// left an empty <c>%TEMP%\tracecommons-sample-&lt;pid&gt;</c>, and one box
/// held about 100 of them.
/// </summary>
public class SampleThrowawayCleanupTests : IDisposable
{
    private readonly string _temp = Path.Combine(Path.GetTempPath(), "tc-throwaway-test-" + Guid.NewGuid().ToString("N"));

    public SampleThrowawayCleanupTests() => Directory.CreateDirectory(_temp);

    public void Dispose()
    {
        try { Directory.Delete(_temp, recursive: true); } catch (IOException) { }
    }

    private string Make(string name, bool withFile = false)
    {
        string dir = Path.Combine(_temp, name);
        Directory.CreateDirectory(dir);
        if (withFile) File.WriteAllText(Path.Combine(dir, "shell-state.json"), "{}");
        return dir;
    }

    [Fact]
    public void TheThrowawayDirectoryIsNamedForThisProcess()
    {
        Assert.Equal(
            Path.Combine(Path.GetTempPath(), SampleDaemon.ThrowawayPrefix + Environment.ProcessId),
            SampleDaemon.ThrowawayConfigDir());
        Assert.Equal(Path.Combine(_temp, "tracecommons-sample-42"), SampleDaemon.ThrowawayConfigDir(_temp, 42));
    }

    [Fact]
    public void ALaunchRemovesTheDirectoriesOfLaunchesThatAreGone()
    {
        string gone = Make("tracecommons-sample-101", withFile: true);
        string goneEmpty = Make("tracecommons-sample-102");
        string running = Make("tracecommons-sample-103");
        string mine = Make("tracecommons-sample-104");

        int removed = SampleDaemon.SweepThrowawayDirs(_temp, currentPid: 104, isRunning: pid => pid == 103);

        Assert.Equal(2, removed);
        Assert.False(Directory.Exists(gone));
        Assert.False(Directory.Exists(goneEmpty));
        Assert.True(Directory.Exists(running), "a running sample launch keeps its directory");
        Assert.True(Directory.Exists(mine), "this launch keeps its own directory");
    }

    [Fact]
    public void TheSweepTouchesNothingButThrowawayDirectories()
    {
        string other = Make("tracecommons-sample-notapid");
        string suffixed = Make("tracecommons-sample-105-keep");
        string unrelated = Make("something-else-106");
        string file = Path.Combine(_temp, "tracecommons-sample-107");
        File.WriteAllText(file, "not a directory");

        int removed = SampleDaemon.SweepThrowawayDirs(_temp, currentPid: 1, isRunning: _ => false);

        Assert.Equal(0, removed);
        Assert.True(Directory.Exists(other));
        Assert.True(Directory.Exists(suffixed));
        Assert.True(Directory.Exists(unrelated));
        Assert.True(File.Exists(file));
    }

    [Fact]
    public void TheSweepSurvivesAMissingTempDirectory()
    {
        Assert.Equal(0, SampleDaemon.SweepThrowawayDirs(Path.Combine(_temp, "absent"), 1, _ => false));
    }

    [Fact]
    public void ExitRemovesThisLaunchsDirectoryWithWhatItHolds()
    {
        string mine = Make("tracecommons-sample-200", withFile: true);

        Assert.True(SampleDaemon.RemoveThrowawayDir(_temp, 200));
        Assert.False(Directory.Exists(mine));
        // Never created (no Settings opened): nothing to do, and no throw.
        Assert.False(SampleDaemon.RemoveThrowawayDir(_temp, 201));
    }

    [Fact]
    public void TheAppSweepsAtLaunchAndRemovesItsOwnOnExit()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Program.cs.txt");
        Assert.True(File.Exists(path), $"Program.cs was not copied to {path}");
        string source = File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);

        int main = source.IndexOf("private static int Main(", StringComparison.Ordinal);
        int start = source.IndexOf("Application.Start(", main, StringComparison.Ordinal);
        int sweep = source.IndexOf("SampleDaemon.SweepThrowawayDirs(", main, StringComparison.Ordinal);
        Assert.True(sweep > main && sweep < start, "the stale directories are swept before the app starts");

        int remove = source.IndexOf("SampleDaemon.RemoveThrowawayDir(", start, StringComparison.Ordinal);
        Assert.True(remove > start, "this launch's directory is removed after the app exits");
        Assert.Contains("AppDomain.CurrentDomain.ProcessExit", source, StringComparison.Ordinal);
    }
}
