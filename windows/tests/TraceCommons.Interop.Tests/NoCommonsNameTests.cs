using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The commons' name stays out of everything this shell shows.
/// </summary>
/// <remarks>
/// <para>
/// Owner rule, 2026-11-02: "The name Trace Commons should currently not
/// appear anywhere in the app ... not in any user visible place." The shared
/// copy this shell decodes is guarded on the Rust side
/// (<c>crates/trace-commons-contributor/tests/no_commons_name_in_copy.rs</c>);
/// this covers the C# and XAML this shell holds itself, and the MSIX
/// manifest's display names.
/// </para>
/// <para>
/// XAML comments and C# comment lines are skipped. <c>TraceCommons</c>
/// namespace and assembly names have no space and are not matched.
/// </para>
/// </remarks>
public class NoCommonsNameTests
{
    /// <summary>
    /// Lines that keep the name, each with why.
    /// </summary>
    /// <remarks>
    /// <c>AutostartCommand.ValueName</c> is the Run-key value's identity, not
    /// only its label: renaming it would leave every existing install's entry
    /// behind, so the app would start twice at login and turning the toggle
    /// off would not remove the old one. Changing it needs a migration, which
    /// is listed for the owner rather than done here.
    /// </remarks>
    private static readonly (string File, string Needle)[] Allowed =
    {
        ("TraceCommons.Interop/AutostartCommand.cs.txt", "public const string ValueName = \"Trace Commons\";"),
    };

    [Fact]
    public void NothingThisShellShowsNamesTheCommons()
    {
        var offences = new List<string>();
        int scanned = 0;
        foreach (string path in SourceFiles())
        {
            scanned++;
            string rel = Path.GetRelativePath(ShellSourceRoot, path).Replace('\\', '/');
            string text = File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
            foreach ((int line, string content) in Findings(text))
            {
                if (Allowed.Any(a => a.File == rel && content.Contains(a.Needle, StringComparison.Ordinal)))
                {
                    continue;
                }
                offences.Add($"{rel}:{line}: {content}");
            }
        }

        Assert.True(scanned > 50, $"scanned only {scanned} files; the shell-source copy moved");
        Assert.True(
            offences.Count == 0,
            "The commons' name must not appear in anything a user sees (owner rule 2026-11-02):\n"
                + string.Join("\n", offences));
    }

    [Fact]
    public void TheScannerFindsTheNameAndSkipsComments()
    {
        Assert.Single(Findings("<TextBlock Text=\"Trace Commons is a pool\" />"));
        Assert.Single(Findings("string s = \"TRACE   COMMONS\";"));
        Assert.Single(Findings("<uap:DisplayName>Trace\n  Commons invite</uap:DisplayName>"));
        Assert.Empty(Findings("<!-- The Trace Commons design system. -->"));
        Assert.Empty(Findings("<!--\n  the Trace Commons mark\n-->"));
        Assert.Empty(Findings("    /// The Trace Commons tray."));
        Assert.Empty(Findings("namespace TraceCommons.Interop;"));
    }

    private static List<(int Line, string Content)> Findings(string text)
    {
        // Blank out XAML/XML comments but keep their newlines, so line
        // numbers stay right.
        string stripped = Regex.Replace(
            text, "<!--.*?-->", m => new string(m.Value.Where(c => c == '\n').ToArray()),
            RegexOptions.Singleline);
        string[] lines = stripped.Split('\n');
        var found = new List<(int, string)>();
        for (int i = 0; i < lines.Length; i++)
        {
            if (IsComment(lines[i]))
            {
                continue;
            }
            string here = Collapse(lines[i]);
            // A name broken across a line wrap in markup or a concatenation.
            string next = i + 1 < lines.Length && !IsComment(lines[i + 1]) ? Collapse(lines[i + 1]) : "";
            bool split = !Names(here) && !Names(next) && Names(here + " " + next);
            if (Names(here) || split)
            {
                found.Add((i + 1, split ? here + " " + next : here));
            }
        }
        return found;
    }

    private static bool IsComment(string line) =>
        line.TrimStart().StartsWith("//", StringComparison.Ordinal);

    private static string Collapse(string line) => Regex.Replace(line.Trim(), @"\s+", " ");

    private static bool Names(string text) =>
        text.Contains("trace commons", StringComparison.OrdinalIgnoreCase);

    private static IEnumerable<string> SourceFiles() =>
        Directory.Exists(ShellSourceRoot)
            ? Directory.EnumerateFiles(ShellSourceRoot, "*.txt", SearchOption.AllDirectories)
            : Array.Empty<string>();

    private static string ShellSourceRoot =>
        Path.Combine(AppContext.BaseDirectory, "shell-source");
}
