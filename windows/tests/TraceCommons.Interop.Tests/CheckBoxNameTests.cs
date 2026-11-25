using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Every check box in the app's markup has an accessibility name. A
/// CheckBox whose content is a panel (a title over a description, as the
/// data-use scopes in Settings are drawn) gets no name from UI Automation,
/// so a screen reader announced the Settings consent ticks as unnamed
/// "check box"es (box UIA dump, 2026-11-20). Each takes the name of its
/// visible title, as the Mac's ConsentScopeToggle does
/// (ConsentSection.swift, accessibilityElement(children: .combine)), with
/// the description as its help text.
/// </summary>
/// <remarks>
/// Read as XAML text, for the reason <see cref="TabStopTests"/> gives: this
/// suite runs on macOS, where no XAML element can be made. The names
/// themselves are witnessed by the box's UIA dump in the PR.
/// </remarks>
public class CheckBoxNameTests
{
    private static string Shell(string file) =>
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", file + ".txt"));

    [Fact]
    public void EveryMarkupCheckBoxIsNamed()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        string[] files = Directory.GetFiles(root, "*.xaml.txt", SearchOption.AllDirectories);
        Assert.NotEmpty(files);
        int seen = 0;
        foreach (string file in files)
        {
            foreach (Match tag in Regex.Matches(File.ReadAllText(file), @"<CheckBox\b[^>]*>"))
            {
                seen++;
                bool named = tag.Value.Contains("AutomationProperties.Name=", StringComparison.Ordinal)
                    || Regex.IsMatch(tag.Value, @"\sContent=""");
                Assert.True(named, $"unnamed CheckBox in {Path.GetFileName(file)}: {tag.Value}");
            }
        }

        // The three consent lists in Settings, at least.
        Assert.True(seen >= 3, $"only {seen} CheckBox tags found");
    }

    [Fact]
    public void ConsentScopesTakeTheirTitleAndDescription()
    {
        string markup = Shell("TraceCommons.App/Controls/SettingsView.xaml");
        MatchCollection boxes = Regex.Matches(markup, @"<CheckBox\b[^>]*>");
        Assert.Equal(3, boxes.Count);
        foreach (Match box in boxes)
        {
            Assert.Contains("AutomationProperties.Name=\"{x:Bind Title}\"", box.Value, StringComparison.Ordinal);
            Assert.Contains("AutomationProperties.HelpText=\"{x:Bind Description}\"", box.Value, StringComparison.Ordinal);
        }
    }

    /// <summary>The go-public acknowledgement is a TextBlock, not a string, so it is named in code.</summary>
    [Fact]
    public void TheGoPublicAcknowledgementIsNamed()
    {
        string source = Shell("TraceCommons.App/Controls/GoPublicDialog.cs");
        Assert.Contains("AutomationProperties.SetName(acknowledgement, PublicProfileCopy.GoPublicAcknowledgement)", source, StringComparison.Ordinal);
    }
}
