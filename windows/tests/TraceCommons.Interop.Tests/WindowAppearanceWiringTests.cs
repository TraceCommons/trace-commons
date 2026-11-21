using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > General's appearance reaches every window that draws the
/// app's own surfaces, as the Mac sets it for the whole app (#2219 review,
/// defect 4): the Monitor, Settings, the session folders window, the
/// preview and About. First run keeps its dark stage.
/// </summary>
public class WindowAppearanceWiringTests
{
    [Theory]
    [InlineData("MainWindow.Appearance.cs")]
    [InlineData("SettingsWindow.xaml.cs")]
    [InlineData("SessionRootsWindow.xaml.cs")]
    [InlineData("PreviewWindow.cs")]
    [InlineData("AboutWindow.cs")]
    public void TheWindowFollowsTheAppearanceChoice(string file)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", file + ".txt");
        Assert.True(File.Exists(path), $"{file} was not copied");
        Assert.Contains("WindowAppearance.Follow(this, ", File.ReadAllText(path));
    }

    [Fact]
    public void FollowingSetsTheChoiceNowAndOnEveryChange()
    {
        string source = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "WindowAppearance.cs.txt"));
        Assert.Contains("root.RequestedTheme = GeneralSection.Theme(AppearanceChoice.Shared.Current);", source);
        Assert.Contains("AppearanceChoice.Shared.Changed += OnChanged;", source);
        Assert.Contains("window.Closed += (_, _) => AppearanceChoice.Shared.Changed -= OnChanged;", source);
    }

    [Fact]
    public void EveryWindowWithASystemTitleBarHasItsCaptionFollowTheTheme()
    {
        string source = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "WindowAppearance.cs.txt"));
        Assert.Contains("if (window is not MainWindow)", source);
        Assert.Contains("WindowCaption.Follow(window, root);", source);
    }

    [Fact]
    public void TheCaptionTakesTheElementsActualThemeAndLetsGoOnClose()
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "WindowCaption.cs.txt");
        Assert.True(File.Exists(path), "WindowCaption.cs was not copied");
        string source = File.ReadAllText(path);
        Assert.Contains("DwmaUseImmersiveDarkMode = 20", source);
        Assert.Contains("root.ActualTheme == ElementTheme.Dark", source);
        Assert.Contains("root.ActualThemeChanged += OnThemeChanged;", source);
        Assert.Contains("root.ActualThemeChanged -= OnThemeChanged;", source);
        Assert.Contains("TaskbarCredit.BaseIcon(", source);
        Assert.Contains("DestroyIcon(", source);
    }
}
