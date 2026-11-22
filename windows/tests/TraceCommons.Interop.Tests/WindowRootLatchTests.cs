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
    public void AClosedWindowIsNeverReadAgain()
    {
        var latch = new WindowRootLatch<Root>(() => throw new InvalidOperationException("closed"));

        latch.MarkClosed();

        Assert.Null(latch.WhileOpen());
        Assert.Null(latch.ForActivation(activating: true));
    }

    [Fact]
    public void AClosedWindowKeepsTheRootItHad()
    {
        var root = new Root();
        var latch = new WindowRootLatch<Root>(() => root);
        latch.WhileOpen();

        latch.MarkClosed();

        Assert.Same(root, latch.WhileOpen());
    }

    /// <summary>
    /// Frames, layout passes and queued callbacks can all land after a
    /// close, so no window reads its content's root except through a latch.
    /// </summary>
    [Fact]
    public void NoWindowReadsItsContentsRootOutsideALatch()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        var offenders = Directory.GetFiles(root, "*.cs.txt", SearchOption.AllDirectories)
            .SelectMany(f => File.ReadAllLines(f).Select(l => (File: Path.GetFileName(f), Line: l.Trim())))
            .Where(x => !x.Line.StartsWith("//", StringComparison.Ordinal))
            .Where(x => x.Line.Contains("Content?.XamlRoot", StringComparison.Ordinal))
            .Where(x => !x.Line.Contains("new WindowRootLatch<XamlRoot>(() => ", StringComparison.Ordinal))
            .Select(x => x.File + ": " + x.Line)
            .ToList();

        Assert.True(offenders.Count == 0, string.Join("\n", offenders));
    }

    /// <summary>The site that crashed a Release build as first run closed.</summary>
    [Fact]
    public void FirstRunsLayoutHandlersStopAtClose()
    {
        string source = Shell("FirstRunWindow.xaml.cs");

        Assert.Matches(@"private void UpdateDragRegions\(\)\s*\{\s*if \(_closed \|\| OpenRoot is null\)", source);
        Assert.Matches(@"private void SizeToCanvas\(\)\s*\{\s*if \(_closed\)", source);

        // The close disposes the painter; a draw already queued must not use it.
        Assert.Matches(@"private void OnDraw\([^)]*\)\s*\{\s*if \(_closed\)", source);
        Assert.Matches(@"_closed = true;\s*_rootLatch\.MarkClosed\(\);", source);
    }

    [Theory]
    [InlineData("SettingsWindow.xaml.cs")]
    [InlineData("AboutWindow.cs")]
    public void AQueuedFocusDoesNotAskAClosedWindow(string file)
    {
        Assert.Contains("() => !_windowClosed && AppWindow.IsVisible", Shell(file));
    }

    [Fact]
    public void TheMainWindowsLateHandlersStopAtClose()
    {
        string shell = Shell("MainWindow.Shell.cs");
        foreach (string method in new[] { "ApplyPaneAvailability", "MovePanes", "LayoutPanes", "ApplyMinimumSize" })
        {
            Assert.Matches(@"private void " + method + @"\([^)]*\)\s*\{\s*if \(_closed\)", shell);
        }

        Assert.Matches(@"_closed = true;\s*_rootLatch\.MarkClosed\(\);", Shell("MainWindow.xaml.cs"));
    }

    [Fact]
    public void TheEdgeRailStopsItsClocksAtClose()
    {
        string source = Shell("EdgeRailWindow.cs");

        Assert.Contains("_rootLatch.MarkClosed();", source);
        Assert.Contains("_closeTimer.Stop();", source);
        Assert.Matches(@"private void Place\(bool open\)\s*\{\s*if \(_windowClosed\)", source);
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

    /// <summary>
    /// An exception nothing caught is recorded, not left to end the process with
    /// no trace; it is survived only for a first-run fault while first run is open.
    /// </summary>
    [Fact]
    public void TheAppRecordsUnhandledExceptionsAndSurvivesOnlyFirstRunFaults()
    {
        string guard = Shell("CrashGuard.cs");
        Assert.Contains("app.UnhandledException +=", guard);
        Assert.Contains("AppDomain.CurrentDomain.UnhandledException +=", guard);
        Assert.Contains("TaskScheduler.UnobservedTaskException +=", guard);
        Assert.Matches(@"if \(FirstRun\.Survives\(e\.Exception\)\)\s*\{\s*e\.Handled = true;", guard);
        Assert.Equal(1, Regex.Matches(guard, @"Handled = true").Count);
        Assert.Contains("CrashGuard.Install(this);", Shell("App.xaml.cs"));
        Assert.Contains("CrashGuard.TrackFirstRun(this);", Shell("FirstRunWindow.xaml.cs"));
    }

    /// <summary>Only the first-run window is counted: no other window passes through the guard.</summary>
    [Theory]
    [InlineData("WindowClocks.cs")]
    [InlineData("SettingsWindow.xaml.cs")]
    [InlineData("AboutWindow.cs")]
    [InlineData("SessionRootsWindow.xaml.cs")]
    [InlineData("PreviewWindow.cs")]
    [InlineData("MainWindow.xaml.cs")]
    public void NoOtherWindowIsCountedAsFirstRun(string file)
    {
        Assert.DoesNotContain("CrashGuard.TrackFirstRun", Shell(file));
    }
}
