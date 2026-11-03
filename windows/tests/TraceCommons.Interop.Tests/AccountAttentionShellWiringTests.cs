using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of the account's "needs attention" status a contributor looks
/// at. <c>TraceCommons.App</c> cannot be referenced from this suite, so these
/// read its sources: the block sits in Settings, every string is bound, and
/// Set up again unenrolls and then starts the first run.
/// </summary>
public class AccountAttentionShellWiringTests
{
    [Fact]
    public void TheBlockIsInSettingsAndEveryStringIsBound()
    {
        string settings = ShellSource("TraceCommons.App/Controls/SettingsView.xaml");
        Assert.Matches(
            new Regex(@"Visibility=""\{x:Bind Settings\.HasAccountAttention, Mode=OneWay\}"">.*?Click=""OnSetUpAgain""", RegexOptions.Singleline),
            settings);
        foreach (string bound in new[]
        {
            "Settings.AccountAttentionTitle", "Settings.AccountAttentionBody",
            "Settings.AccountAttentionPrivateAi", "Settings.AccountAttentionAction",
        })
        {
            Assert.Contains("{x:Bind " + bound + ", Mode=OneWay}", settings, StringComparison.Ordinal);
        }

        // The block is a status in Settings, never a notice in the main window.
        Assert.DoesNotContain("AccountAttention", ShellSource("TraceCommons.App/MainWindow.xaml"), StringComparison.Ordinal);
    }

    [Fact]
    public void SetUpAgainUnenrollsThenStartsTheFirstRun()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        Assert.Contains("DaemonProtocol.Methods.Unenroll", viewModel, StringComparison.Ordinal);
        // Only the three account keys go to the ABI, and only when they changed.
        Assert.Contains("AccountAttentions.Keys(result)", viewModel, StringComparison.Ordinal);
        Assert.Contains("AccountAttentions.For(keys)", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("AccountAttentions.For(result.GetRawText())", viewModel, StringComparison.Ordinal);
        Assert.Matches(
            new Regex(@"if \(_accountAttentionKeysRead && keys == _accountAttentionKeys\)\s*\{\s*return;", RegexOptions.Singleline),
            viewModel);
        Assert.DoesNotContain("\"Set up again\"", viewModel, StringComparison.Ordinal);

        string view = ShellSource("TraceCommons.App/Controls/SettingsView.xaml.cs");
        Assert.Matches(
            new Regex(@"if \(await Settings\.SetUpAgainAsync\(\)\)\s*\{\s*SetUpAgainRequested\?\.Invoke", RegexOptions.Singleline),
            view);
        // One call site for the unenroll, after the confirmation.
        Assert.Single(Regex.Matches(view, @"Settings\.SetUpAgainAsync\("));

        Assert.Contains(
            "settings.SetUpAgainRequested += async (_, _) =>",
            ShellSource("TraceCommons.App/MainWindow.xaml.cs"),
            StringComparison.Ordinal);
        Assert.Equal("unenroll", DaemonProtocol.Methods.Unenroll);
    }

    /// <summary>
    /// "Set up again" asks first, in the Rust's words, and only the
    /// confirmation's primary button reaches the unenroll. Cancel is the
    /// default, so Enter and Escape change nothing.
    /// </summary>
    [Fact]
    public void SetUpAgainIsConfirmedAndOnlyThePrimaryUnenrolls()
    {
        string view = ShellSource("TraceCommons.App/Controls/SettingsView.xaml.cs");
        Match handler = Regex.Match(
            view,
            @"private async void OnSetUpAgain\(object sender, RoutedEventArgs e\)\s*\{(?<body>.*?)\n    \}",
            RegexOptions.Singleline);
        Assert.True(handler.Success, "OnSetUpAgain not found");
        string body = handler.Groups["body"].Value;
        foreach (string bound in new[]
        {
            "Title = Settings.AccountAttentionConfirmTitle",
            "Text = Settings.AccountAttentionConfirmBody",
            "PrimaryButtonText = Settings.AccountAttentionConfirmAction",
            "CloseButtonText = Settings.AccountAttentionConfirmCancel",
            "DefaultButton = ContentDialogButton.Close",
        })
        {
            Assert.Contains(bound, body, StringComparison.Ordinal);
        }

        int asked = body.IndexOf("await DialogGuard.ShowOnceAsync(dialog) != ContentDialogResult.Primary", StringComparison.Ordinal);
        int unenrolled = body.IndexOf("Settings.SetUpAgainAsync()", StringComparison.Ordinal);
        Assert.True(asked >= 0, "the confirmation is not shown through DialogGuard");
        Assert.True(unenrolled > asked, "the unenroll runs before the confirmation answers");
        Assert.Matches(
            new Regex(@"!= ContentDialogResult\.Primary\)\s*\{\s*return;", RegexOptions.Singleline),
            body);
        Assert.DoesNotContain("\"Set up this app again?\"", view, StringComparison.Ordinal);
        Assert.DoesNotContain("\"Cancel\"", body, StringComparison.Ordinal);
    }

    /// <summary>
    /// A refused unenroll shows the Rust's failure line in the block, under
    /// the button, rather than nothing.
    /// </summary>
    [Fact]
    public void ARefusedSetUpAgainShowsTheFailureLine()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        Assert.Matches(
            new Regex(@"if \(response\.IsError\)\s*\{\s*AccountAttentionFailedLine = AccountAttention\?\.Failed \?\? string\.Empty;\s*return false;", RegexOptions.Singleline),
            viewModel);
        Assert.DoesNotContain("\"Nothing changed. Try again.\"", viewModel, StringComparison.Ordinal);

        string settings = ShellSource("TraceCommons.App/Controls/SettingsView.xaml");
        Assert.Matches(
            new Regex(@"Click=""OnSetUpAgain"" />\s*<TextBlock Text=""\{x:Bind Settings\.AccountAttentionFailedLine, Mode=OneWay\}""\s*Visibility=""\{x:Bind Settings\.HasAccountAttentionFailedLine, Mode=OneWay\}""", RegexOptions.Singleline),
            settings);
    }

    /// <summary>
    /// A confirmed Set up again that finds another settings write running
    /// says nothing changed rather than nothing, and the confirmation is not
    /// offered while one runs.
    /// </summary>
    [Fact]
    public void ABusySetUpAgainSaysNothingChanged()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        Assert.Matches(
            new Regex(@"public async Task<bool> SetUpAgainAsync\(\)\s*\{\s*if \(IsBusy\)\s*\{.*?AccountAttentionFailedLine = AccountAttention\?\.Failed \?\? string\.Empty;\s*return false;", RegexOptions.Singleline),
            viewModel);

        string view = ShellSource("TraceCommons.App/Controls/SettingsView.xaml.cs");
        Assert.Matches(
            new Regex(@"if \(_setUpAgainDialogOpen \|\| !Settings\.HasAccountAttention \|\| Settings\.IsBusy\)\s*\{\s*return;", RegexOptions.Singleline),
            view);
    }

    /// <summary>
    /// A confirmed Set up again says what the unenroll left of the NEAR AI
    /// sign-in (#1843), in the Rust's words, before the status is read.
    /// </summary>
    [Fact]
    public void SetUpAgainSaysWhatTheUnenrollLeftOfNearAi()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        Assert.Matches(
            new Regex(@"Methods\.Unenroll.*?NearAiCredentialSurface\.UnenrollNotice\(response, _privateInferenceCopy\).*?Notice = nearAi;.*?Methods\.Status", RegexOptions.Singleline),
            viewModel);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
