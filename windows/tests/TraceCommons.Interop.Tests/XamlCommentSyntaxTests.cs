using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// XAML comments, checked for the one thing this repo's prose style makes
/// easy to get wrong.
/// </summary>
/// <remarks>
/// <para>
/// This repo writes <c>--</c> as an em dash. Every Rust and C# comment in the
/// tree does it, so an agent or a person writing a XAML comment in house style
/// writes it too -- and XML forbids <c>--</c> anywhere inside a comment, and
/// forbids a comment body ending in <c>-</c>. The result is
/// <c>WMC9997: An XML comment cannot contain '--'</c>, which is a BUILD
/// failure, not a warning.
/// </para>
/// <para>
/// It cost a CI round trip on the one job that cannot be run on a macOS or
/// Linux developer machine: WinUI does not build here, so nothing local
/// compiled the markup. The parser stops at the FIRST occurrence, so a file
/// with three of them costs three round trips. This test reads the markup as
/// text instead, which any machine can do, and turns the next one into a red
/// test in seconds.
/// </para>
/// <para>
/// Deliberately not a wording rule and not part of the wording ratchet: it
/// says nothing about what a comment may say, only that it must parse.
/// </para>
/// </remarks>
public class XamlCommentSyntaxTests
{
    [Fact]
    public void NoXamlCommentContainsADoubleHyphen()
    {
        var offences = new List<string>();

        foreach (string path in MarkupFiles())
        {
            string markup = File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);

            foreach (Match comment in Regex.Matches(markup, "<!--(.*?)-->", RegexOptions.Singleline))
            {
                string body = comment.Groups[1].Value;

                foreach (Match hit in Regex.Matches(body, "--"))
                {
                    offences.Add(
                        $"{Path.GetFileName(path)}:{Line(markup, comment.Groups[1].Index + hit.Index)}"
                        + " contains '--' inside a XAML comment. XML forbids it and the build"
                        + " fails with WMC9997. Use a colon, a semicolon, or an em dash character.");
                }

                if (body.EndsWith('-'))
                {
                    offences.Add(
                        $"{Path.GetFileName(path)}:{Line(markup, comment.Index)}"
                        + " has a XAML comment whose body ends with '-', which XML also forbids.");
                }
            }
        }

        Assert.Empty(offences);
    }

    /// <summary>
    /// The guard is worth nothing if it is reading no files, and a copy that
    /// silently stopped happening would look exactly like a pass.
    /// </summary>
    [Fact]
    public void TheMarkupIsActuallyBeingRead()
    {
        List<string> files = MarkupFiles().ToList();
        Assert.NotEmpty(files);
        Assert.Contains(files, path => Path.GetFileName(path) == "MainWindow.xaml.txt");
        Assert.All(files, path => Assert.NotEmpty(File.ReadAllText(path)));
    }

    private static IEnumerable<string> MarkupFiles() =>
        Directory.Exists(ShellSourceRoot)
            ? Directory.EnumerateFiles(ShellSourceRoot, "*.xaml.txt", SearchOption.AllDirectories)
            : Array.Empty<string>();

    private static string ShellSourceRoot =>
        Path.Combine(AppContext.BaseDirectory, "shell-source");

    private static int Line(string text, int index) =>
        text.Take(index).Count(c => c == '\n') + 1;
}
