using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// <c>Window.Close()</c> raises a last deactivation after the window has
/// closed, and reading <c>Window.Content</c> then throws across the ABI,
/// which ends the process with 0xc000027b. So a deactivation answers from
/// what is already known and never reads the window.
/// </summary>
public class WindowRootLatchTests
{
    private sealed class Root
    {
    }

    private static string Shell(string file) => File.ReadAllText(Path.Combine(
        AppContext.BaseDirectory, "shell-source", "TraceCommons.App", file + ".txt"));

    [Fact]
    public void TheLastDeactivationOfAClosedWindowDoesNotReadIt()
    {
        var latch = new WindowRootLatch<Root>(
            () => throw new InvalidOperationException("The window has already been closed."));

        Assert.Null(latch.ForActivation(activating: false));
    }

    [Fact]
    public void ADeactivationAnswersWithTheRootAlreadyKnown()
    {
        var root = new Root();
        bool closed = false;
        var latch = new WindowRootLatch<Root>(
            () => closed ? throw new InvalidOperationException("closed") : root);

        Assert.Same(root, latch.ForActivation(activating: true));
        closed = true;
        Assert.Same(root, latch.ForActivation(activating: false));
        Assert.Same(root, latch.Known);
    }

    [Fact]
    public void AnActivationKeepsLookingUntilTheWindowHasARoot()
    {
        var root = new Root();
        Root? current = null;
        int reads = 0;
        var latch = new WindowRootLatch<Root>(() =>
        {
            reads++;
            return current;
        });

        Assert.Null(latch.ForActivation(activating: true));
        current = root;
        Assert.Same(root, latch.ForActivation(activating: true));
        Assert.Same(root, latch.ForActivation(activating: true));
        Assert.Equal(2, reads);
    }

    [Fact]
    public void WhatIsKnownNeverReadsTheWindow()
    {
        var latch = new WindowRootLatch<Root>(() => throw new InvalidOperationException("closed"));

        Assert.Null(latch.Known);
    }

    [Fact]
    public void TheClocksReadAWindowsContentOnlyThroughTheLatch()
    {
        string source = Shell("WindowClocks.cs");
        string code = string.Join('\n', source.Split('\n').Where(l => !l.TrimStart().StartsWith("//")));

        int latched = Regex.Matches(code, @"new WindowRootLatch<XamlRoot>\(\(\) => window\.Content\?\.XamlRoot\)").Count;
        Assert.True(latched > 0, "the clocks do not read the window through the latch");
        Assert.Equal(latched, Regex.Matches(code, @"window\.Content\?\.XamlRoot").Count);
        Assert.Contains(".ForActivation(", code);
    }
}
