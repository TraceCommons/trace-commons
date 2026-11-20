using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A notice card that never slid has no Translation property, and stopping
/// an animation on it threw at every launch (box, 2026-11-19). The reset
/// gives the card the property before it stops anything (#2219 review,
/// checklist item 3: the crash fix had no test).
/// </summary>
public class NoticeResetMotionTests
{
    [Fact]
    public void TheResetGivesTheCardItsTranslationBeforeStoppingIt()
    {
        string shell = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.Shell.cs.txt"));
        Match reset = Regex.Match(shell, @"private static void ResetMotion\(FrameworkElement card\)\s*\{(.*?)\n    \}", RegexOptions.Singleline);
        Assert.True(reset.Success, "ResetMotion not found");
        string body = reset.Groups[1].Value;
        int enable = body.IndexOf("SetIsTranslationEnabled(card, true);", StringComparison.Ordinal);
        int insert = body.IndexOf("visual.Properties.InsertVector3(\"Translation\"", StringComparison.Ordinal);
        int stop = body.IndexOf("visual.StopAnimation(\"Translation.Y\");", StringComparison.Ordinal);
        Assert.True(enable >= 0 && insert > enable && stop > insert, $"enable {enable}, insert {insert}, stop {stop}");
        Assert.Single(Regex.Matches(body, "SetIsTranslationEnabled"));
    }
}
