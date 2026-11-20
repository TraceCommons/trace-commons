using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings' code-built controls take every brush through the Settings
/// window's styles, whose setters are theme resources (SettingsView.xaml),
/// never by reading a brush key from the application's resources. A brush
/// read in code is one fixed object from the application's theme: it does
/// not follow a live theme change, and it is the wrong one whenever the
/// window's Appearance differs from the application's (W17 review F8,
/// checklist D2).
/// </summary>
public class SettingsThemeBrushTests
{
    private static readonly Regex BrushKey = new("\"Tc[A-Za-z]*Brush\"", RegexOptions.Compiled);

    [Fact]
    public void NoSettingsControlReadsABrushKeyInCode()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls");
        var files = Directory.GetFiles(Path.Combine(root, "Settings"), "*.cs.txt")
            .Concat(Directory.GetFiles(root, "SettingsView*.cs.txt"))
            .ToArray();
        Assert.True(files.Length >= 10, $"only {files.Length} Settings sources were copied");

        var found = new List<string>();
        foreach (string file in files)
        {
            string[] lines = File.ReadAllLines(file);
            for (int i = 0; i < lines.Length; i++)
            {
                if (BrushKey.IsMatch(lines[i]))
                {
                    found.Add($"{Path.GetFileName(file)}:{i + 1}: {lines[i].Trim()}");
                }
            }
        }

        Assert.True(found.Count == 0, "brush keys read in code:\n" + string.Join("\n", found));
    }

    /// <summary>The styles the code takes its brushes through exist, each from a theme resource.</summary>
    [Theory]
    [InlineData("SettingsChipAccentTextStyle", "TcAccentTextBrush")]
    [InlineData("SettingsChipSecondaryTextStyle", "TcInkSecondaryBrush")]
    [InlineData("SettingsChipOnTextStyle", "TcStatusOnTextBrush")]
    [InlineData("SettingsGlassChipStyle", "TcHairlineBrush")]
    [InlineData("SettingsGlassChipMutedStyle", "TcSurfaceInsetBrush")]
    [InlineData("SettingsStatusChipStyle", "TcSurfaceInsetBrush")]
    [InlineData("SettingsStatusDotOnStyle", "TcStatusOnBrush")]
    [InlineData("SettingsStatusDotOffStyle", "TcStatusOffBrush")]
    [InlineData("SettingsSecondaryCaptionStyle", "TcInkSecondaryBrush")]
    [InlineData("SettingsTertiaryCaptionStyle", "TcInkTertiaryBrush")]
    [InlineData("SettingsNoticeTextStyle", "TcInkPrimaryBrush")]
    [InlineData("SettingsNoticeWashAskStyle", "TcStatusAskBrush")]
    [InlineData("SettingsNoticeWashOutsideStyle", "TcStatusOutsideBrush")]
    [InlineData("SettingsNoticeIconAskStyle", "TcStatusAskBrush")]
    [InlineData("SettingsNoticeIconOutsideStyle", "TcStatusOutsideBrush")]
    public void EachCodeStyleTakesItsBrushAsAThemeResource(string style, string brush)
    {
        string xaml = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "SettingsView.xaml.txt"));
        Match block = Regex.Match(xaml, "<Style x:Key=\"" + style + "\"[^>]*>(.*?)</Style>", RegexOptions.Singleline);
        Assert.True(block.Success, $"{style} is not defined in SettingsView.xaml");
        Assert.Contains("{ThemeResource " + brush + "}", block.Groups[1].Value);
    }
}
