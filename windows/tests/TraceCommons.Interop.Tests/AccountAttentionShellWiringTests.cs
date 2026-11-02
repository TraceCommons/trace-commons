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
        Assert.Contains("AccountAttentions.For(result.GetRawText())", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("\"Set up again\"", viewModel, StringComparison.Ordinal);

        string view = ShellSource("TraceCommons.App/Controls/SettingsView.xaml.cs");
        Assert.Matches(
            new Regex(@"if \(await Settings\.SetUpAgainAsync\(\)\)\s*\{\s*SetUpAgainRequested\?\.Invoke", RegexOptions.Singleline),
            view);

        Assert.Contains(
            "settings.SetUpAgainRequested += async (_, _) =>",
            ShellSource("TraceCommons.App/MainWindow.xaml.cs"),
            StringComparison.Ordinal);
        Assert.Equal("unenroll", DaemonProtocol.Methods.Unenroll);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
