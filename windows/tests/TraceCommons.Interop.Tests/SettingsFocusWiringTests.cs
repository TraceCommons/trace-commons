using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Settings cards that are drawn again after a read or a write (the
/// watcher card, the Insights switch, the CLI options) draw through
/// <c>SettingsCards.Replace</c>, which carries keyboard focus
/// (<see cref="Shell.FocusCarry"/>), and read the focused control before the
/// action that may turn it off. The watcher card draws nothing on a status
/// event that changes nothing (W17 review F2; checklist O2, E2).
/// </summary>
public class SettingsFocusWiringTests
{
    [Theory]
    [InlineData("WatchingSection.cs", new[] { "watcher/pause", "watcher/resume", "watcher/retry" })]
    [InlineData("InsightsSection.cs", new[] { "insights/recap", "insights/retry" })]
    [InlineData("CliOptionsCard.cs", new[] { "\"/field\"", "\"/save\"", "cli/retry" })]
    public void ARedrawnCardCarriesFocus(string file, string[] keys)
    {
        string source = Source(file);
        Assert.DoesNotMatch(new Regex(@"(?<![\w.])Content\s*=\s*"), source);
        Assert.Contains("SettingsCards.Replace(this, ", source);
        foreach (string key in keys)
        {
            Assert.Contains(key, source);
        }

        // Every action that redraws reads the focused control first.
        int reads = Regex.Matches(source, @"SettingsCards\.FocusedKey\(this\)").Count;
        int redraws = Regex.Matches(source, @"Render\(at\)").Count;
        Assert.True(redraws > 0 && reads == redraws, $"{file}: {reads} focus reads for {redraws} redraws after an action");
    }

    [Fact]
    public void AStatusEventThatChangesNothingDrawsNothing()
    {
        string source = Source("WatchingSection.cs");
        Assert.Contains("if (Content is not null && Equals(drawn, _drawn))", source);
    }

    private static string Source(string file)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "Settings", file + ".txt");
        Assert.True(File.Exists(path), $"{file} was not copied");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
