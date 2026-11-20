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

        // In the notice stack the card's own Dismiss button goes and the
        // stack's X dismisses it, named with the offer's own word
        // (MonitorNotices.swift:244-247).
        Assert.Matches(
            new Regex(@"<controls:LegacyMigrationOfferView[^>]*x:Name=""LegacyOfferCard""[^>]*ShowsDismiss=""False""", RegexOptions.Singleline),
            main);
        string shell = ShellSource("TraceCommons.App/MainWindow.Shell.cs");
        Assert.Contains("if (ViewModel.LegacyOffer.ShowsCard) yield return (NoticeCatalog.LegacyOffer, LegacyOfferCard);", shell);
        Assert.Contains("ViewModel.LegacyOffer.DismissCard();", shell);
        Assert.Contains("ViewModel.LegacyOffer.Dismiss", shell);
        Assert.Matches(
            new Regex(@"<controls:LegacyMigrationOfferView[^>]*ShowsDismiss=""False""[^>]*LegacyOffer\.IsAvailable", RegexOptions.Singleline),
            settings);
    }

    [Fact]
    public void BothPlacesShareTheMainViewModelsOffer()
    {
        Assert.Contains(
            "new SettingsView(_host, ViewModel.LegacyOffer, ViewModel.UpdateChecks)",
            ShellSource("TraceCommons.App/MainWindow.Shell.cs"));

        string main = ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs");
        Assert.Contains("LegacyOffer.SetStatus(parsedStatus.LegacyInviteMigration, parsedStatus.TenantId)", main);
        Assert.Contains("new NearAiLoginStep(", main);
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
