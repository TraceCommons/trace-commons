using System;
using System.IO;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The watcher's pause and resume words, Settings' empty projects line and
/// the tray's unknown decision count come from the core
/// (<c>preview_copy::monitor_screens_copy</c>,
/// <c>preview_copy::decisions_owed_text</c>), as GTK reads them, and the
/// sites that drew them no longer type them.
/// </summary>
public class MonitorShellCopyTests
{
    private static JsonElement CoreShell()
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_screens_copy_json());
        Assert.False(string.IsNullOrEmpty(json));
        using JsonDocument doc = JsonDocument.Parse(json!);
        return doc.RootElement.GetProperty("shell").Clone();
    }

    [Fact]
    public void TheWatcherWordsAreTheCores()
    {
        JsonElement shell = CoreShell();
        Assert.Equal(shell.GetProperty("pause_watcher").GetString(), MonitorShellCopy.PauseWatcher);
        Assert.Equal(shell.GetProperty("resume_watcher").GetString(), MonitorShellCopy.ResumeWatcher);
        Assert.Equal(shell.GetProperty("projects_empty").GetString(), MonitorShellCopy.ProjectsEmpty);
        Assert.NotEqual(string.Empty, MonitorShellCopy.PauseWatcher);
    }

    [Fact]
    public void AnUnknownDecisionCountIsTheCoresLine()
    {
        string? core = NativeMethods.TakeOwnedString(NativeMethods.tc_decisions_owed_text(-1));
        Assert.False(string.IsNullOrEmpty(core));
        Assert.Equal(core, TrayModel.DecisionCountUnavailable);
        Assert.Equal(core, TrayModel.Compute(null, false, true).MenuHeader);
    }

    /// <summary>
    /// The four sites that typed these words. A literal left behind beside
    /// the core read would render the same today and survive a rename in the
    /// core tomorrow.
    /// </summary>
    [Theory]
    [InlineData("TraceCommons.App/MainWindow.xaml", "Pause watcher")]
    [InlineData("TraceCommons.App/MainWindow.xaml", "Resume watcher")]
    [InlineData("TraceCommons.App/TrayIcon.cs", "Pause watcher")]
    [InlineData("TraceCommons.App/TrayIcon.cs", "Resume watcher")]
    [InlineData("TraceCommons.App/Controls/SettingsView.xaml", "No projects seen yet")]
    [InlineData("TraceCommons.Interop/TrayModel.cs", "Decisions owed unavailable")]
    public void TheSiteNoLongerTypesTheWords(string relativePath, string words)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{path} was not copied");
        Assert.DoesNotContain(words, File.ReadAllText(path), StringComparison.Ordinal);
    }
}
