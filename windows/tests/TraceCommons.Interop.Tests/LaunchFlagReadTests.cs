using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The feature flags are read at launch whatever the window does
/// (AppModel.start -> startDaemon -> refreshAll, AppModel.swift:2085). The
/// window reads them on the daemon host's Started event, so the daemon must
/// start from the window's construction, not its first activation: a launch
/// to the tray, or at login, whose window is never activated, otherwise
/// never reads them and every flag stays off.
/// </summary>
public class LaunchFlagReadTests
{
    [Fact]
    public void TheDaemonStartsAtConstructionNotOnActivation()
    {
        string source = Source();
        int activated = source.IndexOf("private void OnFirstActivated(", StringComparison.Ordinal);
        Assert.True(activated >= 0, "OnFirstActivated is gone");
        string onActivated = source[activated..source.IndexOf("\n    }\n", activated, StringComparison.Ordinal)];
        Assert.DoesNotContain("_host.StartAsync", onActivated, StringComparison.Ordinal);

        int ctor = source.IndexOf("public MainWindow()", StringComparison.Ordinal);
        Assert.True(ctor >= 0, "the constructor is gone");
        string constructor = source[ctor..source.IndexOf("\n    }\n", ctor, StringComparison.Ordinal)];
        Assert.Contains("StartDaemonAtLaunch();", constructor, StringComparison.Ordinal);

        int start = source.IndexOf("private void StartDaemonAtLaunch()", StringComparison.Ordinal);
        Assert.True(start >= 0, "StartDaemonAtLaunch is gone");
        string body = source[start..source.IndexOf("\n    }\n", start, StringComparison.Ordinal)];
        Assert.Contains("LaunchDaemonStart.StartQuietlyAsync(() => _host.StartAsync())", body, StringComparison.Ordinal);
    }

    [Fact]
    public void TheFlagsAreReadWhenTheHostStarts()
    {
        Assert.Contains("_host.Started += () => { _flags.Reset(); _ = _flags.RefreshAsync(); };", Source(), StringComparison.Ordinal);
    }

    private static string Source()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/MainWindow.xaml.cs.txt");
        Assert.True(File.Exists(path), $"MainWindow.xaml.cs was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
