using System;
using System.IO;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The daemon's missions switch: the Mission drafts rail row is shown only
/// when get_settings says exactly true. TraceCommons.App is WinUI and cannot
/// be referenced here, so the view model and markup are read as text.
/// </summary>
public sealed class MissionsSwitchTests
{
    [Theory]
    [InlineData("""{"missions_enabled":true}""", true)]
    [InlineData("""{"missions_enabled":false}""", false)]
    [InlineData("""{"missions_enabled":null}""", false)]
    [InlineData("""{}""", false)]
    public void MissionsAreShownOnlyWhenTheDaemonSaysExactlyTrue(string json, bool shown)
    {
        var settings = JsonSerializer.Deserialize<DaemonSettingsSnapshot>(json);
        Assert.NotNull(settings);
        Assert.Equal(shown, settings!.MissionsShown);
    }

    [Fact]
    public void TheRailRowAndThePaneFollowTheSwitch()
    {
        string markup = Read("MainWindow.xaml.txt");
        string source = Read("MainViewModel.cs.txt");

        Assert.Matches(
            @"Content=""Mission drafts""[^>]*Visibility=""\{x:Bind ViewModel\.MissionsShown, Mode=OneWay\}""",
            markup);
        Assert.Contains("bool missionsShown = settings.MissionsShown;", source, StringComparison.Ordinal);
        Assert.Contains("_missionsShown && _pane == MainPane.MissionDrafts", source, StringComparison.Ordinal);
    }

    private static string Read(string file)
    {
        string path = Path.Combine(AppContext.BaseDirectory, file);
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        return File.ReadAllText(path);
    }
}
