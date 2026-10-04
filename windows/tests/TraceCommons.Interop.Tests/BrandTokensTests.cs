using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The brand accent and status colours, measured on the grounds this shell
/// actually draws them on.
/// </summary>
/// <remarks>
/// <para>
/// <c>Themes/BrandTokens.xaml</c> is generated from
/// <c>design-tokens/glass.tokens.json</c> by
/// <c>scripts/design-tokens/generate.py</c>, and that script's check mode
/// holds it to the JSON. What the generator cannot know is this shell's
/// grounds, which live in <c>DesignSystem.xaml</c>, so contrast is measured
/// here: WCAG 2.1 relative luminance, computed from the committed values,
/// 4.5:1 for text and 3:1 for a fill, rule or indicator, in light and dark.
/// </para>
/// <para>
/// Reads the markup as text, as <see cref="XamlCommentSyntaxTests"/> does,
/// because WinUI does not build off Windows.
/// </para>
/// </remarks>
public class BrandTokensTests
{
    private static readonly string[] Grounds =
    {
        "TcWindowBackgroundBrush",
        "TcSidebarBackgroundBrush",
        "TcChromeBackgroundBrush",
        "TcSurfaceCardBrush",
        "TcSurfaceInsetBrush",
    };

    // Type: every brush a TextBlock or FontIcon takes as its foreground.
    private static readonly string[] TextBrushes =
    {
        "TcAccentTextBrush",
        "TcGreenTextBrush",
        "TcStatusOnTextBrush",
        "TcStatusAskTextBrush",
        "TcGoldTextBrush",
        "TcStatusOutsideTextBrush",
        "TcCoralTextBrush",
    };

    // Fills, rules and indicators drawn straight on a ground: the nav
    // selection bars and progress bars (TcGreenFillBrush), the status hues.
    private static readonly string[] LineBrushes =
    {
        "TcAccentLineBrush",
        "TcGreenFillBrush",
        "TcStatusOnBrush",
        "TcStatusAskBrush",
        "TcGoldBrandBrush",
        "TcStatusOutsideBrush",
        "TcCoralBrandBrush",
    };

    [Theory]
    [InlineData("Light")]
    [InlineData("Default")]
    public void EveryTextBrushClearsFourPointFiveOnEveryGround(string theme)
    {
        Dictionary<string, string> brand = Palette(BrandTokens, theme);
        Dictionary<string, string> system = Palette(DesignSystem, theme);
        var failures = new List<string>();
        foreach (string ink in TextBrushes)
        {
            foreach (string ground in Grounds)
            {
                double ratio = Contrast(brand[ink], system[ground]);
                if (ratio < 4.5)
                {
                    failures.Add($"{theme}: {ink} {brand[ink]} on {ground} {system[ground]} is {ratio:F2}:1");
                }
            }
        }

        Assert.Empty(failures);
    }

    [Theory]
    [InlineData("Light")]
    [InlineData("Default")]
    public void EveryLineBrushClearsThreeOnEveryGround(string theme)
    {
        Dictionary<string, string> brand = Palette(BrandTokens, theme);
        Dictionary<string, string> system = Palette(DesignSystem, theme);
        var failures = new List<string>();
        foreach (string line in LineBrushes)
        {
            foreach (string ground in Grounds)
            {
                double ratio = Contrast(brand[line], system[ground]);
                if (ratio < 3.0)
                {
                    failures.Add($"{theme}: {line} {brand[line]} on {ground} {system[ground]} is {ratio:F2}:1");
                }
            }
        }

        Assert.Empty(failures);
    }

    /// <summary>
    /// The primary action is the accent fill under a white label, and the
    /// label is text. The fill itself is the window ground's and the card's
    /// neighbour, so it is held to 3:1 against those.
    /// </summary>
    [Theory]
    [InlineData("Light")]
    [InlineData("Default")]
    public void ThePrimaryActionIsAMeasuredPair(string theme)
    {
        Dictionary<string, string> brand = Palette(BrandTokens, theme);
        Dictionary<string, string> system = Palette(DesignSystem, theme);

        Assert.Contains("Value=\"{ThemeResource TcAccentBrush}\"", PrimaryButtonStyle());
        Assert.Contains("Value=\"{ThemeResource TcOnAccentBrush}\"", PrimaryButtonStyle());
        Assert.True(Contrast(brand["TcOnAccentBrush"], brand["TcAccentBrush"]) >= 4.5);
        Assert.True(Contrast(brand["TcAccentBrush"], system["TcWindowBackgroundBrush"]) >= 3.0);
        Assert.True(Contrast(brand["TcAccentBrush"], system["TcSurfaceCardBrush"]) >= 3.0);
    }

    [Theory]
    [InlineData("Light")]
    [InlineData("Default")]
    public void TheMarkAccentClearsThreeOnTheMarkField(string theme)
    {
        Dictionary<string, string> brand = Palette(BrandTokens, theme);
        Dictionary<string, string> system = Palette(DesignSystem, theme);
        Assert.True(Contrast(brand["TcMarkAccentBrush"], system["TcMarkFieldBrush"]) >= 3.0);
    }

    /// <summary>
    /// The brand is purple: the old green and mint are gone from the theme,
    /// and the accent keys carry the token values.
    /// </summary>
    [Fact]
    public void TheAccentIsTheBrandPurple()
    {
        Assert.Equal("#6D14F3", Palette(BrandTokens, "Light")["TcAccentBrush"]);
        Assert.Equal("#8A3DFF", Palette(BrandTokens, "Default")["TcAccentBrush"]);
        Assert.Equal("#6D14F3", Palette(BrandTokens, "Light")["TcMarkAccentBrush"]);
        Assert.Equal("#8A3DFF", Palette(BrandTokens, "Default")["TcMarkAccentBrush"]);

        foreach (string retired in new[] { "00D4AA", "178F70", "137C61", "0F7256", "3FBE9A", "5CD3AF" })
        {
            Assert.DoesNotContain(retired, DesignSystem, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain(retired, BrandTokens, StringComparison.OrdinalIgnoreCase);
        }
    }

    /// <summary>
    /// The tray icon's dots are ARGB literals because GDI takes no brush.
    /// They are held to the generated tokens here so they cannot drift.
    /// </summary>
    [Fact]
    public void TheTrayDotsAreTheTokenColours()
    {
        string tray = Source("TrayIcon.cs.txt");
        Dictionary<string, string> light = Palette(BrandTokens, "Light");
        Dictionary<string, string> dark = Palette(BrandTokens, "Default");

        Assert.Contains(
            $"TrayIconState.Attention => lightTaskbar ? {Argb(light["TcAccentBrush"])} : {Argb(dark["TcAccentBrush"])},",
            tray);
        Assert.Contains(
            $"TrayIconState.Unhealthy => lightTaskbar ? {Argb(light["TcStatusOutsideBrush"])} : {Argb(dark["TcStatusOutsideBrush"])},",
            tray);
    }

    [Fact]
    public void EveryBrandKeyHasAHighContrastAlias()
    {
        IEnumerable<string> themed = Palette(BrandTokens, "Light").Keys;
        string highContrast = ThemeBlock(BrandTokens, "HighContrast");
        foreach (string key in themed)
        {
            Assert.Contains($"<StaticResource x:Key=\"{key}\"", highContrast);
        }
    }

    [Fact]
    public void TheDesignSystemMergesTheBrandTokens()
    {
        Assert.Contains("<ResourceDictionary Source=\"ms-appx:///Themes/BrandTokens.xaml\" />", DesignSystem);
    }

    private static string BrandTokens => Source("BrandTokens.xaml.txt");

    private static string DesignSystem => Source("DesignSystem.xaml.txt");

    private static string PrimaryButtonStyle()
    {
        Match style = Regex.Match(
            DesignSystem,
            "<Style x:Key=\"TcPrimaryButtonStyle\".*?</Style>",
            RegexOptions.Singleline);
        Assert.True(style.Success, "TcPrimaryButtonStyle is in DesignSystem.xaml");
        return style.Value;
    }

    private static string Source(string name)
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source");
        string? path = Directory.Exists(root)
            ? Directory.EnumerateFiles(root, name, SearchOption.AllDirectories).FirstOrDefault()
            : null;
        Assert.True(path is not null, $"{name} is copied into the test output");
        return File.ReadAllText(path!).Replace("\r\n", "\n", StringComparison.Ordinal);
    }

    private static string ThemeBlock(string markup, string theme)
    {
        Match block = Regex.Match(
            markup,
            $"<ResourceDictionary x:Key=\"{theme}\">(.*?)</ResourceDictionary>",
            RegexOptions.Singleline);
        Assert.True(block.Success, $"a {theme} theme dictionary");
        return block.Groups[1].Value;
    }

    /// <summary>The opaque brushes of one theme dictionary, key to #RRGGBB.</summary>
    private static Dictionary<string, string> Palette(string markup, string theme)
    {
        var palette = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (Match brush in Regex.Matches(
                     ThemeBlock(markup, theme),
                     "<SolidColorBrush x:Key=\"(\\w+)\" Color=\"#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})\" />"))
        {
            string hex = brush.Groups[2].Value;
            if (hex.Length == 8)
            {
                continue;
            }

            palette[brush.Groups[1].Value] = "#" + hex.ToUpperInvariant();
        }

        Assert.NotEmpty(palette);
        return palette;
    }

    private static string Argb(string hex) => $"0xFF{hex.Substring(1).ToUpperInvariant()}U";

    private static double Contrast(string a, string b)
    {
        double la = Luminance(a);
        double lb = Luminance(b);
        return (Math.Max(la, lb) + 0.05) / (Math.Min(la, lb) + 0.05);
    }

    private static double Luminance(string hex)
    {
        double Channel(int offset)
        {
            double v = int.Parse(hex.AsSpan(offset, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture) / 255.0;
            return v <= 0.04045 ? v / 12.92 : Math.Pow((v + 0.055) / 1.055, 2.4);
        }

        return 0.2126 * Channel(1) + 0.7152 * Channel(3) + 0.0722 * Channel(5);
    }
}
