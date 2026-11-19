using System;
using System.IO;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > Updates over the Windows updater against the Mac's
/// <c>UpdatesSection</c> (StartupSection.swift:184-224, 244-252) and the
/// core's words (shell_words_copy.rs, <c>settings_words</c>).
/// </summary>
public class UpdatesCardTests
{
    private static readonly DateTimeOffset Now = new(2026, 11, 19, 12, 0, 0, TimeSpan.Zero);

    [Fact]
    public void AnUnpackagedCopyIsTheMacsDisabledBranch()
    {
        UpdatesCard card = UpdatesCard.Live(packaged: false, version: "0.40.1", lastCheck: Now);
        Assert.Equal("Version", card.VersionLabel);
        Assert.Equal("0.40.1", card.Version);
        Assert.Equal("Updates unavailable", card.Tag);
        Assert.False(card.TagOn);
        Assert.Null(card.LastChecked);
        Assert.Equal("Updates are turned off for this build.", card.Caption);
        Assert.Null(card.CheckNow);
    }

    /// <summary>
    /// The owner's Windows wording (2026-11-19): the schedule App Installer
    /// keeps and when it installs, from the core's Windows-only keys.
    /// </summary>
    [Fact]
    public void APackagedCopySaysTheWindowsScheduleInTheCoresWords()
    {
        UpdatesCard card = UpdatesCard.Live(packaged: true, version: "0.40.1", lastCheck: null);
        Assert.Equal("Checks every 8 hours", card.Tag);
        Assert.True(card.TagOn);
        Assert.Equal("Installs updates when the app is closed or next opens.", card.Caption);
        Assert.Equal("Check Now", card.CheckNow);
        Assert.Equal(SettingsLegacyWords.Word("checks_every_eight_hours"), card.Tag);
        Assert.Equal(SettingsLegacyWords.Word("installs_when_closed"), card.Caption);
    }

    [Fact]
    public void TheLastCheckedLineIsHiddenUntilACheckHasCompleted()
    {
        UpdatesCard card = UpdatesCard.Of(true, "1", SettingsLegacyWords.Word, lastCheck: null, now: Now);
        Assert.Null(card.LastChecked);
    }

    [Fact]
    public void TheLastCheckedLineIsTheMacsSentenceTimedFromTheAppsOwnCheck()
    {
        UpdatesCard card = UpdatesCard.Of(true, "1", SettingsLegacyWords.Word, lastCheck: Now.AddHours(-3), now: Now);
        Assert.Equal("Last checked 3 hours ago.", card.LastChecked);
        UpdatesCard fresh = UpdatesCard.Of(true, "1", SettingsLegacyWords.Word, lastCheck: Now, now: Now);
        Assert.Equal("Last checked just now.", fresh.LastChecked);
    }

    [Fact]
    public void AMissingWordDrawsNothingInItsPlace()
    {
        UpdatesCard card = UpdatesCard.Of(false, "1", _ => string.Empty, lastCheck: null, now: Now);
        Assert.Null(card.Tag);
        Assert.Null(card.Caption);
        UpdatesCard packaged = UpdatesCard.Of(true, "1", _ => string.Empty, lastCheck: Now, now: Now);
        Assert.Null(packaged.CheckNow);
        Assert.Null(packaged.Tag);
        Assert.Null(packaged.Caption);
        Assert.Null(packaged.LastChecked);
    }

    [Fact]
    public void CheckNowThatCompletesUpdatesTheLine()
    {
        var clock = new UpdateCheckClock();
        int changed = 0;
        clock.Changed += () => changed++;
        Assert.Null(UpdatesCard.Of(true, "1", SettingsLegacyWords.Word, clock.LastCompleted, Now).LastChecked);

        // The launch check, then Check Now three hours later.
        clock.Record(TcUpdateAvailability.NoUpdates, Now.AddHours(-3));
        Assert.Equal("Last checked 3 hours ago.", UpdatesCard.Of(true, "1", SettingsLegacyWords.Word, clock.LastCompleted, Now).LastChecked);
        clock.Record(TcUpdateAvailability.Available, Now);
        Assert.Equal("Last checked just now.", UpdatesCard.Of(true, "1", SettingsLegacyWords.Word, clock.LastCompleted, Now).LastChecked);
        Assert.Equal(2, changed);
    }

    [Theory]
    [InlineData(TcUpdateAvailability.NoUpdates, true)]
    [InlineData(TcUpdateAvailability.Available, true)]
    [InlineData(TcUpdateAvailability.Required, true)]
    [InlineData(TcUpdateAvailability.Error, false)]
    [InlineData(TcUpdateAvailability.Unknown, false)]
    public void OnlyACheckThatGotAnAnswerIsRecorded(TcUpdateAvailability availability, bool recorded)
    {
        var clock = new UpdateCheckClock();
        int changed = 0;
        clock.Changed += () => changed++;
        clock.Record(availability, Now);
        Assert.Equal(recorded ? Now : null, clock.LastCompleted);
        Assert.Equal(recorded ? 1 : 0, changed);
    }

    /// <summary>
    /// The app's own checks feed the line: the launch check and Check Now
    /// both go through <c>CheckForUpdateAsync</c>, which records its answer
    /// on the main view model's clock; Settings is built over that clock,
    /// and the Updates card redraws when it moves and when the kept window
    /// is shown again.
    /// </summary>
    [Fact]
    public void LaunchAndCheckNowRecordOnTheClockSettingsDrawsFrom()
    {
        string main = ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs");
        Assert.Contains(
            "TcUpdateAvailability availability = await _updater.CheckAsync().ConfigureAwait(true);\n        UpdateChecks.Record(availability, DateTimeOffset.UtcNow);",
            main);
        Assert.Contains("public UpdateCheckClock UpdateChecks { get; } = new();", main);

        string window = ShellSource("TraceCommons.App/MainWindow.Shell.cs");
        Assert.Contains("new SettingsView(_host, ViewModel.LegacyOffer, ViewModel.UpdateChecks)", window);
        Assert.Contains("settings.CheckForUpdatesRequested += async (_, _) => await ViewModel.CheckForUpdateAsync();", window);
        Assert.Contains("_settingsView?.RefreshUpdates();", window);

        string section = ShellSource("TraceCommons.App/Controls/Settings/UpdatesSection.cs");
        Assert.Contains("UpdatesCard.Live(_packaged, _version, _checks.LastCompleted)", section);
        Assert.Contains("_checks.Changed += Draw;", section);
    }

    [Fact]
    public void AFailedCheckKeepsTheLastCompletedTime()
    {
        var clock = new UpdateCheckClock();
        clock.Record(TcUpdateAvailability.NoUpdates, Now.AddHours(-3));
        clock.Record(TcUpdateAvailability.Error, Now);
        Assert.Equal(Now.AddHours(-3), clock.LastCompleted);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
