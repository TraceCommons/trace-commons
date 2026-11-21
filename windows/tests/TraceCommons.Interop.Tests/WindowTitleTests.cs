using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Every window Windows can name in its switchers names the app, "Near AI
/// Gateway" from <c>brand.rs</c> (owner rule 2026-11-20: "the app should
/// refer to itself in Windows for the navigation, switcher etc as Near AI
/// Gateway").
/// </summary>
/// <remarks>
/// A WinUI <c>Window</c> cannot be built off a desktop, so the App half is
/// held at the source: every class deriving from <c>Window</c> sets its
/// <c>Title</c> from <see cref="WindowTitles"/>, never from a literal or a
/// word of its own. The values are then checked here, at run time, against
/// <c>tc_app_name</c>. A Window class with no Title at all is what put
/// "WinUI Desktop" in Alt+Tab, and is a failure too.
/// </remarks>
public class WindowTitleTests
{
    /// <summary>The windows that stand for the whole app: the name, exactly.</summary>
    [Fact]
    public void TheAppsOwnWindowsAreTitledWithTheName()
    {
        Assert.Equal("Near AI Gateway", WindowTitles.Main);
        Assert.Equal(MonitorWords.AppName, WindowTitles.Main);
        Assert.Equal(MonitorWords.AppName, WindowTitles.FirstRun);
        Assert.Equal(MonitorWords.AppName, WindowTitles.SessionRoots);
        Assert.Equal(MonitorWords.AppName, WindowTitles.TrayFlyout);
    }

    /// <summary>
    /// A secondary window: its own word, then the name, so the switcher
    /// shows whose window it is and never two identical entries.
    /// </summary>
    [Fact]
    public void SecondaryWindowsCarryTheirWordAndTheName()
    {
        Assert.Equal("Settings - Near AI Gateway", WindowTitles.Settings);
        Assert.Equal("Look inside - Near AI Gateway", WindowTitles.Preview);
        Assert.Equal("About Near AI Gateway", WindowTitles.About);
        Assert.Equal(MonitorWords.SettingsTitle + " - " + MonitorWords.AppName, WindowTitles.Settings);
        Assert.Equal(MonitorWords.LookInside + " - " + MonitorWords.AppName, WindowTitles.Preview);
    }

    [Fact]
    public void AMissingWordLeavesTheNameAloneNotASeparator()
    {
        Assert.Equal(MonitorWords.AppName, WindowTitles.Secondary(string.Empty));
        Assert.Equal("X - " + MonitorWords.AppName, WindowTitles.Secondary("X"));
    }

    /// <summary>
    /// Every title a switcher can show names the app; the edge rail, a tool
    /// window never in a switcher, is the one title that does not.
    /// </summary>
    [Fact]
    public void EveryTitleButTheRailsNamesTheApp()
    {
        foreach ((string window, string title) in Titles().Where(t => t.Window != nameof(WindowTitles.EdgeRail)))
        {
            Assert.False(string.IsNullOrEmpty(title), window);
            Assert.Contains(MonitorWords.AppName, title, StringComparison.Ordinal);
            Assert.DoesNotContain("Trace Commons", title, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("Near.ai", title, StringComparison.OrdinalIgnoreCase);
        }

        Assert.Equal("Edge rail", WindowTitles.EdgeRail);
    }

    /// <summary>
    /// The windows the App defines, each with the <see cref="WindowTitles"/>
    /// member it must set. A new Window class fails here until it is added,
    /// with a title that names the app.
    /// </summary>
    private static readonly Dictionary<string, string> Expected = new()
    {
        ["MainWindow"] = nameof(WindowTitles.Main),
        ["FirstRunWindow"] = nameof(WindowTitles.FirstRun),
        ["SessionRootsWindow"] = nameof(WindowTitles.SessionRoots),
        ["TrayFlyoutWindow"] = nameof(WindowTitles.TrayFlyout),
        ["AboutWindow"] = nameof(WindowTitles.About),
        ["SettingsWindow"] = nameof(WindowTitles.Settings),
        ["PreviewWindow"] = nameof(WindowTitles.Preview),
        ["EdgeRailWindow"] = nameof(WindowTitles.EdgeRail),
    };

    [Fact]
    public void EveryWindowClassSetsItsTitleFromWindowTitles()
    {
        Dictionary<string, List<string>> found = TitleAssignments();

        Assert.Equal(Expected.Keys.OrderBy(k => k), found.Keys.OrderBy(k => k));
        var offences = new List<string>();
        foreach ((string window, List<string> assignments) in found)
        {
            string want = "WindowTitles." + Expected[window];
            if (assignments.Count == 0)
            {
                offences.Add($"{window}: never sets Title (Alt+Tab shows a blank or \"WinUI Desktop\")");
            }

            foreach (string rhs in assignments.Where(a => a != want))
            {
                offences.Add($"{window}: Title = {rhs}; want {want}");
            }
        }

        Assert.True(offences.Count == 0, "Window titles come from WindowTitles:\n" + string.Join("\n", offences));
    }

    [Fact]
    public void TheScannerFindsStatementsNotInitializers()
    {
        Assert.Equal(new[] { "\"Look inside\"" }, Statements("        Title = \"Look inside\";\n"));
        Assert.Equal(new[] { "WindowTitles.Main" }, Statements("    Title = WindowTitles.Main;"));
        Assert.Empty(Statements("            Title = prompt.Title,\n"));
        Assert.Empty(Statements("    // Title = \"x\";"));
    }

    private static IEnumerable<(string Window, string Title)> Titles() => new[]
    {
        (nameof(WindowTitles.Main), WindowTitles.Main),
        (nameof(WindowTitles.FirstRun), WindowTitles.FirstRun),
        (nameof(WindowTitles.SessionRoots), WindowTitles.SessionRoots),
        (nameof(WindowTitles.TrayFlyout), WindowTitles.TrayFlyout),
        (nameof(WindowTitles.About), WindowTitles.About),
        (nameof(WindowTitles.Settings), WindowTitles.Settings),
        (nameof(WindowTitles.Preview), WindowTitles.Preview),
        (nameof(WindowTitles.EdgeRail), WindowTitles.EdgeRail),
    };

    private static readonly Regex WindowClass =
        new(@"\bclass\s+(\w+)\s*:\s*Window\b", RegexOptions.Compiled);

    /// <summary>A <c>Title = ...;</c> statement on a line of its own; an object initializer's <c>Title = ...,</c> is not one.</summary>
    private static readonly Regex TitleStatement =
        new(@"^\s*Title\s*=\s*(.+?);\s*$", RegexOptions.Compiled | RegexOptions.Multiline);

    private static IEnumerable<string> Statements(string text) =>
        TitleStatement.Matches(text).Select(m => m.Groups[1].Value.Trim());

    /// <summary>
    /// Each Window class in the App's source, with the right-hand side of
    /// every <c>Title</c> statement in the files that declare it (a partial
    /// class spans several).
    /// </summary>
    private static Dictionary<string, List<string>> TitleAssignments()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        Assert.True(Directory.Exists(root), $"no shell-source copy at {root}");
        var texts = Directory.EnumerateFiles(root, "*.cs.txt", SearchOption.AllDirectories)
            .Select(p => File.ReadAllText(p).Replace("\r\n", "\n", StringComparison.Ordinal))
            .ToList();

        var result = new Dictionary<string, List<string>>();
        foreach (string text in texts)
        {
            foreach (Match m in WindowClass.Matches(text))
            {
                result[m.Groups[1].Value] = new List<string>();
            }
        }

        foreach (string window in result.Keys.ToList())
        {
            var declares = new Regex(@"\bclass\s+" + window + @"\b");
            foreach (string text in texts.Where(t => declares.IsMatch(t)))
            {
                result[window].AddRange(Statements(text));
            }
        }

        return result;
    }
}
