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

    /// <summary>
    /// The app's name comes from <c>brand.rs</c> (<c>tc_app_name</c>); the
    /// name it shipped under before (<c>brand::FORMER_APP_NAMES</c>,
    /// "Near.ai") appears nowhere a person sees: XAML text, window titles,
    /// automation names, C# strings and the manifest (W10, checklist J1).
    /// A URL's host (<c>cloud.near.ai</c>) is not the name.
    /// </summary>
    [Fact]
    public void NothingThisShellShowsUsesTheFormerName()
    {
        var offences = new List<string>();
        foreach (string path in SourceFiles())
        {
            string rel = path == ManifestCopy
                ? "manifest-source/Package.appxmanifest.txt"
                : Path.GetRelativePath(ShellSourceRoot, path).Replace('\\', '/');
            string text = File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
            foreach ((int line, string content) in Findings(text, NamesFormer))
            {
                offences.Add($"{rel}:{line}: {content}");
            }
        }

        Assert.True(
            offences.Count == 0,
            "The app's name comes from brand.rs (tc_app_name), never the former \"Near.ai\" literal:\n"
                + string.Join("\n", offences));
    }

    [Fact]
    public void TheFormerNameScannerFindsTheNameAndSkipsHostsAndComments()
    {
        Assert.Single(Findings("<Window Title=\"Near.ai\">", NamesFormer));
        Assert.Single(Findings("AutomationProperties.Name=\"Open Near.ai\"", NamesFormer));
        Assert.Single(Findings("string s = $\"NEAR.AI — {detail}\";", NamesFormer));
        Assert.Empty(Findings("string url = \"https://cloud.near.ai/dashboard\";", NamesFormer));
        Assert.Empty(Findings("string url = \"near.ai/login\";", NamesFormer));
        // A bare host compared in code is not the name: lower case, quoted
        // whole (HomeFacts' link check on the W11 branch).
        Assert.Empty(Findings("|| !string.Equals(actionUri!.Host, \"near.ai\", StringComparison.OrdinalIgnoreCase)", NamesFormer));
        Assert.Single(Findings("string title = \"Near.ai\";", NamesFormer));
        Assert.Single(Findings("<TextBlock Text=\"near.ai\" />", NamesFormer));
        Assert.Empty(Findings("<!-- the Near.ai window -->", NamesFormer));
        Assert.Empty(Findings("    /// The Near.ai tray.", NamesFormer));
    }

    /// <summary>
    /// The former name, in any case, except as a URL's host or path and
    /// except a bare host passed to a call: the lower-case literal
    /// <c>"near.ai"</c> as an argument (after <c>(</c> or <c>,</c>), as a
    /// link check compares it. The same literal anywhere else, an attribute
    /// or an assignment, is still the name.
    /// </summary>
    private static bool NamesFormer(string text) =>
        Regex.IsMatch(
            Regex.Replace(text, @"(?<=[(,]\s*)""near\.ai""", "\"\""),
            @"(?<![\w./-])near\.ai(?![\w/])",
            RegexOptions.IgnoreCase);

    private static List<(int Line, string Content)> Findings(string text) => Findings(text, Names);

    private static List<(int Line, string Content)> Findings(string text, Func<string, bool> names)
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
            bool split = !names(here) && !names(next) && names(here + " " + next);
            if (names(here) || split)
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
        (Directory.Exists(ShellSourceRoot)
            ? Directory.EnumerateFiles(ShellSourceRoot, "*.txt", SearchOption.AllDirectories)
            : Array.Empty<string>())
        .Append(ManifestCopy);

    private static string ShellSourceRoot =>
        Path.Combine(AppContext.BaseDirectory, "shell-source");

    /// <summary>
    /// The MSIX manifest, copied beside <c>shell-source/</c> rather than into
    /// it so the wording ratchet does not count it. Reading a missing file
    /// throws, so a copy that stopped happening fails here.
    /// </summary>
    private static string ManifestCopy =>
        Path.Combine(AppContext.BaseDirectory, "manifest-source", "Package.appxmanifest.txt");
}
