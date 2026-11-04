using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of changing the NEAR AI login (F10) a contributor looks at.
/// <c>TraceCommons.App</c> cannot be referenced from this suite, so these
/// read its sources: the three entry points sit in Settings' account section,
/// every string is bound from the core's copy, the dialog draws the driver's
/// page, and Switch is never the default button.
/// </summary>
public class AccountLoginChangeShellWiringTests
{
    [Fact]
    public void TheThreeEntryPointsAreInSettingsAndEveryStringIsBound()
    {
        string settings = ShellSource("TraceCommons.App/Controls/SettingsView.xaml");
        foreach (string bound in new[]
        {
            "Settings.HasLoginMismatch", "Settings.LoginMismatchBody", "Settings.LoginMismatchSignIn",
            "Settings.LoginMismatchChange", "Settings.ShowsLoginMismatchChange",
            "Settings.ShowsLoginChangeButton", "Settings.LoginChangeLabel",
            "Settings.ShowsPayoutsNeedLogin", "Settings.PayoutsNeedsLoginBody", "Settings.PayoutsNeedsLoginAdd",
            "Settings.LoginChangeStartFailedLine", "Settings.HasLoginChangeStartFailedLine",
        })
        {
            Assert.Contains("{x:Bind " + bound + ", Mode=OneWay}", settings, StringComparison.Ordinal);
        }

        // The mismatch block: body, Sign in, and the change only while held.
        Assert.Matches(
            new Regex(
                @"Visibility=""\{x:Bind Settings\.HasLoginMismatch, Mode=OneWay\}"">.*?LoginMismatchBody.*?Click=""OnLoginMismatchSignIn"".*?"
                + @"Visibility=""\{x:Bind Settings\.ShowsLoginMismatchChange, Mode=OneWay\}"".*?Click=""OnUseDifferentAccount""",
                RegexOptions.Singleline),
            settings);
        // All three entry points open the one dialog.
        Assert.Equal(3, Regex.Matches(settings, @"Click=""OnUseDifferentAccount""").Count);

        // A status in Settings, never a notice in the main window.
        string main = ShellSource("TraceCommons.App/MainWindow.xaml");
        Assert.DoesNotContain("LoginMismatch", main, StringComparison.Ordinal);
        Assert.DoesNotContain("LoginChange", main, StringComparison.Ordinal);
    }

    [Fact]
    public void TheViewModelReadsTheStatusKeysAndTheCoreCopy()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        Assert.Contains("AccountLoginChangeStatus.From(result)", viewModel, StringComparison.Ordinal);
        Assert.Contains("AccountChangeCopy.Copy()", viewModel, StringComparison.Ordinal);
        // Read at every place the attention block is.
        Assert.Equal(
            Regex.Matches(viewModel, @"FillAccountAttention\((status|statusResponse)\);").Count,
            Regex.Matches(viewModel, @"FillLoginChange\((status|statusResponse)\);").Count);
        // The entry-point rules are the interop's, ported from macOS.
        Assert.Contains("_loginChangeStatus.OffersChange", viewModel, StringComparison.Ordinal);
        Assert.Contains("_loginChangeStatus.ShowsChangeButton", viewModel, StringComparison.Ordinal);
        Assert.Contains("_loginChangeStatus.ShowsPayoutsLine", viewModel, StringComparison.Ordinal);
        // The wallet field's label is the wallet join's own.
        Assert.Contains("_witnessCopy?.Wallet?.Account", viewModel, StringComparison.Ordinal);
        // A confirmed change re-reads the account at once.
        Assert.Contains("LoginChange.Finished += async (_, _) => await RereadAccountAsync()", viewModel, StringComparison.Ordinal);
        // Sign in is the NEAR AI ceremony Private AI uses.
        Assert.Matches(
            new Regex(@"public async Task MismatchSignInAsync\(\).*?Methods\.NearAiCredentialStart.*?CredentialBrowser\.ContinueAsync\(", RegexOptions.Singleline),
            viewModel);
    }

    [Fact]
    public void TheDialogDrawsThePageAndSwitchIsNeverTheDefault()
    {
        string dialog = ShellSource("TraceCommons.App/Controls/AccountLoginChangeDialog.cs");
        foreach (string drawn in new[]
        {
            "_dialog.Title = page.Title",
            "_heading.Text = page.Heading",
            "foreach (string line in page.Lines)",
            "page.AsksWalletAccount",
            "_error.Text = page.Error",
            "_dialog.CloseButtonText = page.Cancel",
            "_dialog.PrimaryButtonText = page.Primary",
            "_dialog.IsPrimaryButtonEnabled = _flow.CanPressPrimary",
            "NamePrimaryButton(page.PrimaryAccessibilityName)",
            "AutomationProperties.SetName(primary, name)",
        })
        {
            Assert.Contains(drawn, dialog, StringComparison.Ordinal);
        }

        Assert.Contains(
            "_dialog.DefaultButton = page.IsDefault ? ContentDialogButton.Primary : ContentDialogButton.None;",
            dialog,
            StringComparison.Ordinal);
        // The primary button never closes the dialog itself; Cancel and
        // Escape end the ceremony in the daemon.
        Assert.Matches(new Regex(@"OnPrimary\(.*?args\.Cancel = true;.*?_flow\.PrimaryAsync\(\)", RegexOptions.Singleline), dialog);
        Assert.Matches(new Regex(@"OnClosing\(.*?_flow\.CancelAsync\(\)", RegexOptions.Singleline), dialog);
        Assert.Contains("DialogGuard.ShowOnceAsync(dialog._dialog)", dialog, StringComparison.Ordinal);
        Assert.DoesNotContain("Text = \"", dialog, StringComparison.Ordinal);

        string view = ShellSource("TraceCommons.App/Controls/SettingsView.xaml.cs");
        Assert.Matches(
            new Regex(@"private async void OnUseDifferentAccount\(.*?await Settings\.LoginChange\.BeginAsync\(\);\s*await AccountLoginChangeDialog\.ShowAsync\(XamlRoot, Settings\.LoginChange, Settings\.WalletAccountLabel\);", RegexOptions.Singleline),
            view);
    }

    [Fact]
    public void ThePrivateAiCardSaysTheMismatchInTheApprovedWords()
    {
        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");
        Assert.Contains("{x:Bind ViewModel.LoginMismatchLine, Mode=OneWay}", markup, StringComparison.Ordinal);
        Assert.Contains("{x:Bind ViewModel.HasLoginMismatchLine, Mode=OneWay}", markup, StringComparison.Ordinal);

        string codeBehind = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml.cs");
        Assert.Contains("AccountLoginChangeSurface.MismatchLine(", codeBehind, StringComparison.Ordinal);
        Assert.Contains("_host.StatusChanged += ReadLoginMismatch;", codeBehind, StringComparison.Ordinal);
    }

    [Fact]
    public void NoXamlCommentAddedHereHoldsADoubleHyphen()
    {
        foreach (string file in new[] { "TraceCommons.App/Controls/SettingsView.xaml", "TraceCommons.App/Controls/PrivateInferenceView.xaml" })
        {
            string text = ShellSource(file);
            foreach (Match comment in Regex.Matches(text, "<!--(?<body>.*?)-->", RegexOptions.Singleline))
            {
                Assert.DoesNotContain("--", comment.Groups["body"].Value, StringComparison.Ordinal);
            }
        }
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
