using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > General draws the Mac's cards in the Mac's order
/// (GeneralSection.swift:26-31): Appearance, the edge rail, the Traces
/// map's style, promotions, then credit arrivals.
/// The promotions switch is the core's words over the shell preference Home
/// reads (#2219 review, defect 5).
/// </summary>
public class GeneralSectionCardsTests
{
    [Fact]
    public void GeneralHoldsTheMacsCardsInItsOrder()
    {
        string xaml = Source("Controls/SettingsView.xaml");
        int body = xaml.IndexOf("x:Name=\"BodyGeneral\"", StringComparison.Ordinal);
        int end = xaml.IndexOf("x:Name=\"RuleStartup\"", body, StringComparison.Ordinal);
        Assert.True(body > 0 && end > body);
        string general = xaml[body..end];
        int appearance = general.IndexOf("x:Name=\"GeneralHost\"", StringComparison.Ordinal);
        int rail = general.IndexOf("x:Name=\"EdgeRailSwitch\"", StringComparison.Ordinal);
        int map = general.IndexOf("x:Name=\"MapStyleHost\"", StringComparison.Ordinal);
        int promotions = general.IndexOf("x:Name=\"PromotionsSwitch\"", StringComparison.Ordinal);
        int credit = general.IndexOf("x:Name=\"CreditAnimationSwitch\"", StringComparison.Ordinal);
        Assert.True(appearance >= 0 && appearance < rail && rail < map && map < promotions && promotions < credit,
            $"appearance {appearance}, rail {rail}, map {map}, promotions {promotions}, credit {credit}");

        // Each a card, as the Mac's GlassEyebrowCard.
        foreach (string card in new[] { "EdgeRailCard", "MapStyleCard", "PromotionsCard", "CreditAnimationCard" })
        {
            Assert.Contains($"<Border x:Name=\"{card}\" Style=\"{{StaticResource SettingsCardStyle}}\">", general);
        }
    }

    [Fact]
    public void ThePromotionsSwitchIsTheCoresWordsOverTheHomePreference()
    {
        string shell = Source("Controls/SettingsView.Shell.cs");
        Assert.Contains("MonitorShellCopy.Section(\"home_history\", \"promotions_title\")", shell);
        Assert.Contains("MonitorShellCopy.Section(\"home_history\", \"promotions_toggle\")", shell);
        Assert.Contains("MonitorShellCopy.Section(\"home_history\", \"promotions_caption\")", shell);
        Assert.Contains("prefs.ShowsPromotions);", shell);
        Assert.Contains("ShellPreferences.Shared.ShowsPromotions = PromotionsSwitch.IsOn;", shell);
        Assert.Contains("ShowHome();", Source("MainWindow.Home.cs"));
    }

    [Fact]
    public void ThePromotionsWordsAreTheCores()
    {
        Assert.Equal("Home", MonitorShellCopy.Section("home_history", "promotions_title"));
        Assert.Equal("Show promotions", MonitorShellCopy.Section("home_history", "promotions_toggle"));
    }

    private static string Source(string relative) => File.ReadAllText(Path.Combine(
        AppContext.BaseDirectory, "shell-source", "TraceCommons.App", relative + ".txt"));
}
