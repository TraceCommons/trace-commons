using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of the legacy invite move a contributor looks at.
/// <c>TraceCommons.App</c> cannot be referenced from this suite, so these read
/// its sources: that the one offer control is drawn in both places, that both
/// share the one view model, and that no sentence is typed in the markup.
/// </summary>
public class LegacyMigrationShellWiringTests
{
    [Fact]
    public void TheNoticesCardCanBeDismissedAndSettingsCannot()
    {
        string main = ShellSource("TraceCommons.App/MainWindow.xaml");
        string settings = ShellSource("TraceCommons.App/Controls/SettingsView.xaml");

        Assert.Matches(
            new Regex(@"<controls:LegacyMigrationOfferView[^>]*ShowsDismiss=""True""[^>]*ViewModel\.LegacyOffer\.ShowsCard", RegexOptions.Singleline),
            main);
        Assert.Matches(
            new Regex(@"<controls:LegacyMigrationOfferView[^>]*ShowsDismiss=""False""[^>]*LegacyOffer\.IsAvailable", RegexOptions.Singleline),
            settings);
    }

    [Fact]
    public void BothPlacesShareTheMainViewModelsOffer()
    {
        Assert.Contains(
            "new SettingsView(_host, ViewModel.LegacyOffer)",
            ShellSource("TraceCommons.App/MainWindow.xaml.cs"));

        string main = ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs");
        Assert.Contains("LegacyOffer.SetStatus(parsedStatus.LegacyInviteMigration, parsedStatus.TenantId)", main);
        Assert.Contains("new NearAiPasskeyStep(", main);
        Assert.Contains("move.Settled += () => _ = RefreshAsync();", main);
    }

    [Fact]
    public void NoSentenceIsTypedInTheOfferMarkupOrItsViewModel()
    {
        string xaml = ShellSource("TraceCommons.App/Controls/LegacyMigrationOfferView.xaml");
        // Every Text and Content is a binding.
        foreach (Match match in Regex.Matches(xaml, @"\b(Text|Content)=""([^""]*)"""))
        {
            Assert.StartsWith("{x:Bind", match.Groups[2].Value, StringComparison.Ordinal);
        }

        string viewModel = ShellSource("TraceCommons.App/ViewModels/LegacyMigrationOfferViewModel.cs");
        foreach (string typed in new[] { "\"Move\"", "\"Dismiss\"", "\"Cancel\"", "\"Got it\"" })
        {
            Assert.DoesNotContain(typed, viewModel, StringComparison.Ordinal);
        }
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
