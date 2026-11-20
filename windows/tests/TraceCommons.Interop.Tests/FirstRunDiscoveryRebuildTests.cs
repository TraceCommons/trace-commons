using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Tools step is built when the window opens, before discovery has
/// answered. The step must be built again once it has, whatever else
/// changed: before this, only a status reply with <c>logged_in: true</c>
/// (the normalDay sample) changed the content key, so on a computer that is
/// not signed in (the empty sample, a fresh install) the tools found never
/// appeared as rows (box tc-win-az9, 0 of 4 runs).
/// </summary>
public class FirstRunDiscoveryRebuildTests
{
    [Fact]
    public void TheContentKeyFollowsDiscovery()
    {
        string source = Source();
        int key = source.IndexOf("private string ContentKey(FirstRunState state)", StringComparison.Ordinal);
        Assert.True(key >= 0, "ContentKey is gone");
        string body = source[key..source.IndexOf(";\n", key, StringComparison.Ordinal)];
        Assert.Contains("_discoveries", body, StringComparison.Ordinal);
    }

    [Fact]
    public void LoadingCountsADiscovery()
    {
        string source = Source();
        int load = source.IndexOf("private async Task LoadAsync()", StringComparison.Ordinal);
        Assert.True(load >= 0, "LoadAsync is gone");
        string body = source[load..source.IndexOf("\n    }\n", load, StringComparison.Ordinal)];
        int probe = body.IndexOf("SourceDiscovery.ProbeThisMachine", StringComparison.Ordinal);
        int counted = body.IndexOf("_discoveries++", StringComparison.Ordinal);
        Assert.True(probe >= 0 && counted > probe, "a discovery is counted after the probe answers");
    }

    private static string Source()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/FirstRunWindow.xaml.cs.txt");
        Assert.True(File.Exists(path), $"FirstRunWindow.xaml.cs was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
