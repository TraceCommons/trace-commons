using System;
using System.IO;
using System.Linq;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>The Private AI guide card (spec W19, InferenceViews.swift:369-405).</summary>
public sealed class PrivateAIGuideTests
{
    private static string TempPath() => PrivateAIGuide.ClosedPath(Path.Combine(Path.GetTempPath(), "tc-guide-" + Guid.NewGuid().ToString("N")));

    // The managed table's words, in the Mac's step order (InferenceViews.swift:382-386).
    [Fact]
    public void TheCardIsTheManagedTablesWordsInTheMacsOrder()
    {
        var guide = new PrivateAIGuide(PrivateAIGuide.ManagedWords(), TempPath());
        Assert.True(guide.IsShown);
        Assert.Equal("How Private AI works", guide.Title);
        Assert.NotEqual(string.Empty, guide.Lead);
        Assert.Equal(new[] { 1, 2, 3 }, guide.Steps.Select(s => s.Number));
        Assert.Equal("Earn credit", guide.Steps[0].Title);
        Assert.All(guide.Steps, s => Assert.NotEqual(string.Empty, s.Title));
        Assert.All(guide.Steps, s => Assert.NotEqual(string.Empty, s.Line));
        Assert.Equal(MonitorShellCopy.Top("close"), guide.CloseName);
        Assert.NotEqual(string.Empty, guide.CloseName);
    }

    // Closed with its X, it stays closed on this computer (@AppStorage).
    [Fact]
    public void ClosedItStaysClosed()
    {
        string path = TempPath();
        var guide = new PrivateAIGuide(PrivateAIGuide.ManagedWords(), path);
        guide.Close();
        Assert.False(guide.IsShown);
        Assert.False(new PrivateAIGuide(PrivateAIGuide.ManagedWords(), path).IsShown);
        Directory.Delete(Path.GetDirectoryName(path)!, true);
    }
}
