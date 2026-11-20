using System;
using System.IO;
using System.Linq;
using System.Xml.Linq;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A <c>&lt;StaticResource&gt;</c> alias outside a theme dictionary resolves
/// once, in the theme the app started in. The tab strip's and the toolbar's
/// checked-state overrides were such aliases, so after a live switch the
/// selected tab's title was drawn in the old theme's ink: 1.23:1, blank to
/// the eye (box live switch, tc-win-az9, 2026-11-20). Inside a Default, Light
/// or HighContrast dictionary each theme resolves its own.
/// </summary>
/// <remarks>
/// Reads the XAML as text (checklist B2): the only other witness is a live
/// theme switch on the test box, which CI cannot make, and this keeps the
/// next such alias from coming back between box runs. The box's live plan
/// (windows/scripts/a11y/plans/live.txt) is the behavioural check.
/// </remarks>
public class ThemeResourceAliasTests
{
    private static readonly XNamespace X = "http://schemas.microsoft.com/winfx/2006/xaml";

    [Fact]
    public void EveryResourceAliasSitsInAThemeDictionary()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        string[] files = Directory.GetFiles(root, "*.xaml.txt", SearchOption.AllDirectories);
        Assert.NotEmpty(files);
        var outside = files
            .SelectMany(file => XDocument.Load(file).Descendants()
                .Where(e => e.Name.LocalName == "StaticResource")
                .Where(e => !e.Ancestors().Any(a => a.Name.LocalName == "ResourceDictionary"
                    && (string?)a.Attribute(X + "Key") is "Default" or "Light" or "Dark" or "HighContrast"))
                .Select(e => $"{Path.GetFileName(file)}: {(string?)e.Attribute(X + "Key")}"))
            .ToList();
        Assert.Empty(outside);
    }
}
